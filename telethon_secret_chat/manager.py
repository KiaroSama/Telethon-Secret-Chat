"""The object an application owns - contracts/public-api.md §1-§3.

An object the application CONSTRUCTS, not attributes patched onto its
``TelegramClient``. contracts/public-api.md: "Patching makes the dependency
invisible to a reader and unmockable to a test - and the archived base package did
exactly that."

The manager is the orchestrator and holds no protocol rules of its own. The rules
live in the modules named for the sections they implement - ``dh`` for §1.2,
``handshake`` for §1.3-§1.6, ``crypto`` for §2, ``framing`` for §3.1-§3.4,
``sequence`` for §3.5-§3.8, ``actions`` for §5 - and this file wires them to
Telethon and to the storage backend the application chose.

Telethon touch points, all public except one, kept in one place so a canary test can
pin them (FR-016, research.md Q3): ``client(...)`` for the TL requests,
``client.add_event_handler``/``remove_event_handler`` for the update subscription,
``client.get_input_entity``, and ``client._parse_message_text`` - private, with the
documented fallback of accepting pre-parsed entities from the caller.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
import secrets
import time
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

from telethon.extensions import BinaryReader
from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, dh, files, framing, handshake, rekey as rekey_module, sequence
from .chat import ChatState, SecretChat
from .errors import SecretChatError, StorageRequired
from .events import (
    ChatClosedEvent,
    ChatReady,
    ChatRequested,
    DecryptFailed,
    MessageReceived,
    ServiceActionReceived,
)
from .schema import secret_tl as tl
from .storage import StorageBackend

__all__ = ["SecretChatManager"]

# This package's own logger, never the host client's. research.md Q3: "Principle IV
# requires this package to control what it emits, so borrowing the host's logger was
# never right." §8.8 measured the archived package logging plaintext through the
# borrowed one at DEBUG.
log = logging.getLogger("telethon_secret_chat")


class SecretChatManager:
    """contracts/public-api.md §1.

    ``storage`` is required and has no default. FR-014: "a library that quietly
    writes key material somewhere is a library that writes it somewhere the operator
    did not protect."
    """

    def __init__(self, client, storage: Optional[StorageBackend]):
        if storage is None:
            raise StorageRequired()
        self._client = client
        self._storage = storage
        self._chats: Dict[int, SecretChat] = {}
        self._secrets: Dict[int, Dict[str, int]] = {}  # chat_id -> the DH scratch
        self._history: Dict[int, List[MessageReceived]] = {}
        self._handlers: Dict[str, List[Callable]] = {}
        self._running = False
        # Bound once. `self._on_update` builds a NEW bound-method object on every
        # attribute access, so subscribing with one and unsubscribing with another
        # leaves the subscription in place on any client that compares by identity.
        # Holding the reference makes `stop` work regardless of which comparison the
        # client uses.
        self._subscription = self._on_update

    # --- lifecycle ------------------------------------------------------------

    async def start(self) -> None:
        """Subscribe to the encryption updates. Idempotent (contracts §1)."""
        if self._running:
            return
        for chat_id in self._storage.list():
            record = self._storage.load(chat_id)
            if record is not None:
                self._chats[chat_id] = SecretChat.from_record(record)
        self._client.add_event_handler(self._subscription)
        self._running = True

    async def stop(self) -> None:
        """Unsubscribe and flush. Idempotent (contracts §1)."""
        if not self._running:
            return
        self._client.remove_event_handler(self._subscription)
        for chat in self._chats.values():
            self._save(chat)
        self._running = False

    def on(self, event: str, handler: Callable) -> None:
        """Register a handler for one of the seven event names of contracts §3."""
        self._handlers.setdefault(event, []).append(handler)

    def _emit(self, event: Any) -> None:
        """Hand one event to its handlers, awaiting the ones that need awaiting.

        An ``async def`` handler called synchronously returns a coroutine that
        nobody runs: Python warns "coroutine ... was never awaited" into stderr and
        the handler's whole body simply does not happen. That is the worst shape a
        failure can take here, because the caller sees a registered handler and a
        clean run - measured on a real chat, where an application accepting
        invitations in an async handler left every one of them `pending` for ever.

        And handlers ARE naturally async: the interesting ones accept a chat, answer
        a message or write a file, and every one of those awaits. So a coroutine is
        scheduled on the running loop rather than dropped.

        Scheduled, not awaited: ``_emit`` is called from inside the update path, and
        awaiting a handler there would let an application's slow or hanging handler
        stall the decryption of every other chat. A handler that raises is logged
        with its own name, never re-raised into the update loop.
        """
        for handler in self._handlers.get(type(event).__name__, []):
            try:
                result = handler(event)
            except Exception:
                log.exception(
                    "secret-chat handler %r failed", getattr(handler, "__name__", handler)
                )
                continue
            if inspect.isawaitable(result):
                asyncio.ensure_future(self._run_handler(handler, result))

    @staticmethod
    async def _run_handler(handler, awaitable) -> None:
        """Await one scheduled handler, so a failure is reported rather than lost.

        Without this, a coroutine handed to ``ensure_future`` that raises reports
        "Task exception was never retrieved" at garbage-collection time, minutes
        later, with no clue which handler it was.
        """
        try:
            await awaitable
        except Exception:
            log.exception("secret-chat handler %r failed", getattr(handler, "__name__", handler))

    def _save(self, chat: SecretChat) -> None:
        """data-model.md §5: "save is called on a meaningful state change, not on
        every attribute write" - §8 measured the archived package saving inside
        ``__setattr__``, a write amplifier and a partial-write hazard."""
        self._storage.save(chat.to_record())

    # --- the nine operations (contracts §2) -----------------------------------

    async def create(self, user) -> SecretChat:
        """§1.3. Validates the server's parameters before anything is derived."""
        g, p = await self._dh_config()
        a = handshake.generate_secret()
        g_a = handshake.public_value(g, a, p)
        result = await self._client(
            functions.messages.RequestEncryptionRequest(
                user_id=await self._client.get_input_entity(user),
                random_id=secrets.randbits(31),
                g_a=g_a.to_bytes(256, "big"),
            )
        )
        chat = SecretChat(
            id=result.id,
            access_hash=result.access_hash,
            peer_user_id=getattr(result, "participant_id", None) or int(user),
            is_outbound=True,
            admin_id=getattr(result, "admin_id", None),
            participant_id=getattr(result, "participant_id", None),
        )
        chat.dh_prime, chat.dh_g = p, g
        self._chats[chat.id] = chat
        self._secrets[chat.id] = {"secret": a, "p": p, "g": g}
        self._save(chat)
        return chat

    async def accept(self, chat_id: int) -> SecretChat:
        """§1.4. B computes the key immediately and publishes its fingerprint."""
        chat = self._require(chat_id)
        pending = self._secrets.get(chat_id, {})
        g_a = pending.get("g_a")
        if g_a is None:
            raise KeyError(f"no pending request for chat {chat_id}")
        g, p = pending["g"], pending["p"]

        b = handshake.generate_secret()
        key = handshake.shared_key(peer_value=g_a, secret=b, p=p, chat_id=chat_id)
        g_b = handshake.public_value(g, b, p)

        chat.dh_prime, chat.dh_g = p, g
        chat.adopt_key(key)
        await self._client(
            functions.messages.AcceptEncryptionRequest(
                peer=types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash),
                g_b=g_b.to_bytes(256, "big"),
                key_fingerprint=chat.key_fingerprint,
            )
        )
        self._secrets[chat_id] = {"secret": b, "p": p, "g": g}
        self._save(chat)
        self._emit(ChatReady(chat.id, chat.peer_user_id, chat.key_fingerprint))
        await self._notify_layer(chat)
        return chat

    async def close(self, chat_id: int, reason: str = "closed by this application") -> None:
        """Terminal. Closing an already-closed chat states it and does not raise."""
        chat = self._require(chat_id)
        if chat.state is ChatState.CLOSED:
            return
        try:
            await self._client(
                functions.messages.DiscardEncryptionRequest(chat_id=chat.id, delete_history=False)
            )
        except Exception:
            # The chat is over locally whatever the server says; a transport error
            # here must not leave a chat that believes it is still usable.
            log.debug("discardEncryption failed for chat %s; closing locally anyway", chat_id)
        chat.close(reason)
        self._save(chat)
        self._emit(ChatClosedEvent(chat.id, reason))

    def list(self) -> List[SecretChat]:
        return list(self._chats.values())

    def status(self, chat_id: int) -> SecretChat:
        return self._require(chat_id)

    async def send_message(
        self, chat_id: int, text: str, entities=None, reply_to: Optional[int] = None
    ) -> int:
        """contracts §2. Resolves when TELEGRAM accepts the ciphertext, not when the
        peer acknowledges - acknowledgement arrives as ``MessageAcknowledged``.

        ``reply_to`` is the ``random_id`` of the message being replied to - the
        encrypted layer has no message ids, so a reply points at the random id the
        sender chose (§7.1's ``reply_to_random_id``). It is NOT checked against this
        side's history: the peer's own sent messages never pass through it, so a check
        here would refuse every reply to something they said.
        """
        chat = self._require(chat_id)
        chat.require_sendable()
        random_id = secrets.randbits(63)
        if entities is None and text:
            text, entities = await self._parse_text(text)
        # §4.1's trigger, checked on the path that counts messages. Before the send
        # rather than after, so the message that crosses the threshold already goes
        # out under whichever key the exchange settles on.
        await self._rekey_if_due(chat)
        message = tl.DecryptedMessage(
            random_id=random_id,
            ttl=chat.ttl,
            message=text,
            entities=entities or None,
            reply_to_random_id=reply_to,
        )
        await self._send(chat, message)
        return random_id

    async def rekey(self, chat_id: int) -> None:
        """§4. Ask for a new key now, rather than waiting for §4.1's trigger.

        The spec's Assumptions: "the package rekeys automatically on the documented
        trigger, and the application can ask for one. It is not left to the caller
        to remember."
        """
        await rekey_module.start(self, self._sendable(chat_id))

    async def _rekey_if_due(self, chat: SecretChat) -> None:
        """§4.1's trigger, checked where messages are counted."""
        if chat.state is ChatState.READY and rekey_module.should_rekey(chat, time.time()):
            await rekey_module.start(self, chat)

    # --- files (§6, contracts §2) ---------------------------------------------
    # The bodies live in `files.py`, which owns §6 end to end: the one-time keys,
    # the MD5 fingerprint that is NOT §1.5's, and the ordering FR-012 requires.
    # Keeping them there rather than here is what stops §6 being half in a module
    # named for it and half in the orchestrator.

    async def send_file(
        self,
        chat_id: int,
        path,
        *,
        caption: str = "",
        mime_type=None,
        kind=None,
        reply_to: Optional[int] = None,
    ) -> int:
        """§6.3-§6.4. The key travels inside the message, the address outside it.

        ``kind`` is one of ``files.MEDIA_KINDS``; absent, it is inferred from the
        file. A kind the file cannot be is refused before anything is uploaded, and
        so is a caption on one of the two kinds that carry none.
        """
        return await files.send(
            self, self._sendable(chat_id), path, caption, mime_type, kind, reply_to
        )

    async def save_file(self, message: MessageReceived, path) -> Path:
        """§6.3: check the fingerprint, THEN write. FR-012."""
        return await files.receive(self, message, path)

    # --- the service actions the consumer needs (§5, FR-010) ------------------

    async def set_ttl(self, chat_id: int, seconds: int) -> None:
        """§5.1. Stored and transmitted; the countdown is not enforced locally.

        §5 marks the exact moment a countdown starts UNVERIFIED for every media
        kind, and the spec's Assumptions take the default of "stores and transmits
        the TTL without enforcing it locally" rather than inventing a rule an
        official client might not share.
        """
        chat = self._require(chat_id)
        chat.require_sendable()
        chat.ttl = seconds
        await self._send_action(chat, actions_module.set_message_ttl(seconds))

    async def mark_read(self, chat_id: int, random_ids) -> None:
        """§5.2."""
        await self._send_action(self._sendable(chat_id), actions_module.read_messages(random_ids))

    async def delete_messages(self, chat_id: int, random_ids) -> None:
        """§5.3, with §3.8's rewrite of anything not yet acknowledged.

        "securely destroy the contents of the message", "change the local copy of
        the original message to decryptedMessageActionDeleteMessages with random_id
        equal to its own random_id", then "create a new outgoing message deleting
        the original message" - because a retained message that simply vanished
        would make the peer's resend request unanswerable, and an unanswerable
        resend ends the chat (§3.7).
        """
        chat = self._sendable(chat_id)
        self._rewrite_retained_as_deletes(chat, set(random_ids))
        await self._send_action(chat, actions_module.delete_messages(random_ids))

    async def screenshot(self, chat_id: int, random_ids) -> None:
        """§5.4."""
        await self._send_action(
            self._sendable(chat_id), actions_module.screenshot_messages(random_ids)
        )

    async def flush_history(self, chat_id: int) -> None:
        """§5.5."""
        await self._send_action(self._sendable(chat_id), actions_module.flush_history())

    async def set_typing(self, chat_id: int, action=None) -> None:
        """§5.8."""
        await self._send_action(self._sendable(chat_id), actions_module.typing(action))

    def _sendable(self, chat_id: int) -> SecretChat:
        chat = self._require(chat_id)
        chat.require_sendable()
        return chat

    async def _send_action(self, chat: SecretChat, action) -> None:
        """Every outbound action goes through one place, so §3.1's "any service
        messages in secret chats must also increment the seq_no" is structural
        rather than remembered."""
        await self._send(
            chat,
            tl.DecryptedMessageService(random_id=secrets.randbits(63), action=action),
        )

    def _rewrite_retained_as_deletes(self, chat: SecretChat, random_ids: set) -> None:
        """§3.8, on the retention queue.

        The retained copy keeps its ``out_seq_no`` - the hole it would leave is
        exactly what §3.8 exists to prevent - and loses its content, so a later
        resend replays a self-delete rather than the plaintext the user asked to
        destroy.
        """
        remaining = self._storage.retained_out(chat.id)
        self._storage.drop_out(chat.id, max((m["seq_no"] for m in remaining), default=0))
        for item in remaining:
            with BinaryReader(bytes.fromhex(item["body"])) as reader:
                wrapper = tl.read_object(reader)
            inner = wrapper.message
            if getattr(inner, "random_id", None) in random_ids:
                wrapper.message = tl.DecryptedMessageService(
                    random_id=inner.random_id,
                    action=actions_module.delete_messages([inner.random_id]),
                )
                item = {"seq_no": item["seq_no"], "body": bytes(wrapper).hex()}
            self._storage.queue_out(chat.id, item)

    def read_history(self, chat_id: int, limit: int = 50) -> List[MessageReceived]:
        self._require(chat_id)
        return self._history.get(chat_id, [])[-limit:]

    # --- sending --------------------------------------------------------------

    async def _send(self, chat: SecretChat, message, file=None) -> None:
        """Wrap (§3.1), count (§3.4), encrypt (§2) and hand to Telegram.

        The counters are assigned here and nowhere else. §3.4: "assign in_seq_no and
        out_seq_no to each message at the exact moment when the message is created,
        and never change them in the future."
        """
        wrapped = framing.wrap(
            message,
            layer=framing.outgoing_layer(chat.layer),
            in_seq_no=framing.transform_in_seq_no(chat.in_seq_no, chat.is_outbound),
            out_seq_no=framing.transform_out_seq_no(chat.out_seq_no, chat.is_outbound),
        )
        frame = crypto.encrypt_frame(chat.key, bytes(wrapped), chat.out_x)
        # §3.4: "incremented strictly by 1 after any message (service or not) is
        # sent/received and processed" - service messages included, which is why
        # every outgoing path goes through this one function.
        chat.out_seq_no += 1
        chat.messages_since_rekey += 1
        self._storage.queue_out(
            chat.id,
            {"seq_no": wrapped.out_seq_no, "body": bytes(wrapped).hex()},
        )
        self._save(chat)
        peer = types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash)
        if file is None:
            request = functions.messages.SendEncryptedRequest(
                peer=peer, random_id=secrets.randbits(63), data=frame
            )
        else:
            # §6.4: a media message goes through a different method, carrying the
            # file's address alongside the ciphertext rather than inside it.
            request = functions.messages.SendEncryptedFileRequest(
                peer=peer, random_id=secrets.randbits(63), data=frame, file=file
            )
        await self._client(request)

    async def _answer_any_resend(self, chat: SecretChat, wrapper) -> None:
        """§3.7's one exception to in-order interpretation.

        "decryptedMessageActionResend must always be interpreted immediately upon
        receipt in all cases, even if its out_seq_no>=C+1." Otherwise the rule is
        circular: a request that arrives after the hole it is trying to close would
        be queued BEHIND that hole, and the hole would never close.

        "Note that each decryptedMessageActionResend must only be handled once, it
        must not be interpreted again when we interpret messages in the queue." The
        action is rewritten to Noop in place, exactly as TDLib does it - and because
        the rewrite happens before `sequence.accept`, the copy that goes into the
        gap queue carries the Noop too.
        """
        inner = getattr(wrapper, "message", None)
        if not isinstance(inner, (tl.DecryptedMessageService, tl.DecryptedMessageService8)):
            return
        action = inner.action
        if not isinstance(action, tl.DecryptedMessageActionResend):
            return
        # Raises ResendUnsatisfiable - and closes the chat - when it cannot be
        # served, which is what the protocol requires rather than silence.
        answer = sequence.answer_resend(
            chat, self._storage, action.start_seq_no, action.end_seq_no
        )
        inner.action = tl.DecryptedMessageActionNoop()
        for retained in answer:
            await self._resend_retained(chat, retained)

    async def _resend_retained(self, chat: SecretChat, retained: dict) -> None:
        """Put a retained message back on the wire unchanged.

        The original bytes, so the original ``out_seq_no`` - which is the only thing
        that can fill the peer's hole. Nothing is counted: §3.4 says the numbers are
        assigned once "at the exact moment when the message is created" and are
        never changed. §8.3 measured the archived package composing a NEW message
        here, with a new counter, which leaves the peer's hole exactly where it was.
        """
        frame = crypto.encrypt_frame(chat.key, bytes.fromhex(retained["body"]), chat.out_x)
        await self._client(
            functions.messages.SendEncryptedRequest(
                peer=types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash),
                random_id=secrets.randbits(63),
                data=frame,
            )
        )

    async def _notify_layer(self, chat: SecretChat) -> None:
        """§7.3: "As soon as a new secret chat has been created, immediately after
        the secret key has been successfully exchanged"."""
        await self._send(
            chat,
            tl.DecryptedMessageService(
                random_id=secrets.randbits(63),
                action=actions_module.notify_layer(framing.MAX_LAYER),
            ),
        )

    # --- receiving ------------------------------------------------------------

    async def _on_update(self, update) -> None:
        """The one subscription. Everything the server pushes about secret chats
        arrives here, and nothing raises out of it - contracts §3."""
        try:
            if isinstance(update, types.UpdateEncryption):
                await self._on_encryption(update.chat)
            elif isinstance(update, types.UpdateNewEncryptedMessage):
                await self._on_encrypted_message(update.message)
        except SecretChatError as failure:
            # A protocol refusal is the application's business, as an event.
            chat_id = getattr(failure, "chat_id", None)
            self._emit(DecryptFailed(chat_id or 0, getattr(failure, "reason", "refused")))

    async def _on_encryption(self, encrypted) -> None:
        """The chat-state updates of §1.3-§1.4."""
        if isinstance(encrypted, types.EncryptedChatRequested):
            g, p = await self._dh_config()
            g_a = dh.value_from_bytes(encrypted.g_a)
            dh.check_peer_value(g_a, p, chat_id=encrypted.id)
            chat = SecretChat(
                id=encrypted.id,
                access_hash=encrypted.access_hash,
                peer_user_id=encrypted.admin_id,
                is_outbound=False,
                admin_id=encrypted.admin_id,
                participant_id=encrypted.participant_id,
            )
            chat.dh_prime, chat.dh_g = p, g
            self._chats[chat.id] = chat
            self._secrets[chat.id] = {"g_a": g_a, "p": p, "g": g}
            self._save(chat)
            self._emit(ChatRequested(chat.id, encrypted.admin_id))

        elif isinstance(encrypted, types.EncryptedChat):
            chat = self._chats.get(encrypted.id)
            if chat is None or chat.key is not None:
                return  # not ours, or already established
            pending = self._secrets[chat.id]
            g_b = dh.value_from_bytes(encrypted.g_a_or_b)
            key = handshake.shared_key(
                peer_value=g_b, secret=pending["secret"], p=pending["p"], chat_id=chat.id
            )
            try:
                # §1.4: on a mismatch "messages.discardEncryption must be executed
                # and the user notified" - the chat is not usable and is not kept.
                handshake.verify_fingerprint(
                    chat_id=chat.id, key=key, claimed=encrypted.key_fingerprint
                )
            except SecretChatError as failure:
                await self.close(chat.id, failure.reason)
                raise
            chat.adopt_key(key)
            self._save(chat)
            self._emit(ChatReady(chat.id, chat.peer_user_id, chat.key_fingerprint))
            await self._notify_layer(chat)

        elif isinstance(encrypted, types.EncryptedChatDiscarded):
            chat = self._chats.get(encrypted.id)
            if chat is not None and chat.state is not ChatState.CLOSED:
                chat.close("the peer discarded the chat")
                self._save(chat)
                self._emit(ChatClosedEvent(chat.id, "the peer discarded the chat"))

    async def _on_encrypted_message(self, message) -> None:
        """Decrypt (§2.7), check the counters (§3.4-§3.6), dispatch (§5).

        Every refusal along the way becomes a ``DecryptFailed`` event and stops;
        nothing partially-validated reaches the application (FR-006).
        """
        chat = self._chats.get(message.chat_id)
        if chat is None or chat.key is None:
            self._emit(DecryptFailed(message.chat_id, "no key is held for this chat"))
            return
        if chat.state is ChatState.CLOSED:
            self._emit(DecryptFailed(chat.id, "the chat is closed"))
            return
        # §2.6 and §4.5: the fingerprint prefix says WHICH held key this frame was
        # written under - the current one, the previous one still retained through a
        # rekey, or the pending one when the peer has switched and its CommitKey has
        # not arrived yet.
        named = rekey_module.select_key(
            chat, int.from_bytes(message.bytes[:8], "little", signed=True)
        )
        if named is None:
            self._emit(DecryptFailed(chat.id, "no key this chat holds matches the frame"))
            return
        if named is chat.pending_key:
            # §4.5: "a message encrypted by the new key, recognized by the value of
            # key_fingerprint ... it assumes that A has started using the new key for
            # encryption, and does the same" - the switch, without the CommitKey.
            rekey_module.adopt_new_key(chat, named)
            named = chat.key
            if chat.state is ChatState.REKEYING:
                chat.transition_to(ChatState.READY)
        try:
            body = crypto.decrypt_frame(
                chat.key if named is chat.key else named, message.bytes, chat.in_x, chat_id=chat.id
            )
            wrapper = framing.unwrap(body, chat_id=chat.id)
            await self._answer_any_resend(chat, wrapper)
            accepted = sequence.accept(chat, wrapper, self._storage)
        except SecretChatError as refusal:
            # Includes the failures §3.4 and §3.6 say must END the chat: `sequence`
            # closes it and raises, and the application learns both facts as events.
            self._emit(DecryptFailed(chat.id, getattr(refusal, "reason", "refused")))
            if chat.state is ChatState.CLOSED:
                self._emit(ChatClosedEvent(chat.id, chat.closed_reason or "refused"))
            self._save(chat)
            return

        # §3.6's echo says what the peer has taken in, so retention can shrink.
        sequence.forget_acknowledged(chat, self._storage, chat.peer_in_seq_no)
        # §4.8: with no gap open, nothing written before the switch can still be in
        # flight, so the old key has no remaining use and is dropped.
        rekey_module.retire_previous_key_if_settled(chat)
        for item in accepted.ready:
            await self._deliver(chat, item, message)
        self._save(chat)
        if accepted.resend is not None:
            # §3.7: ask for exactly the missing span, once per hole.
            await self._send(
                chat,
                tl.DecryptedMessageService(
                    random_id=secrets.randbits(63),
                    action=actions_module.resend(*accepted.resend),
                ),
            )

    async def _deliver(self, chat: SecretChat, wrapper, envelope) -> None:
        """One accepted message, in conversation order."""
        inner = wrapper.message
        # The layer was raised in `sequence.accept`, beside the §3.6 check that
        # forbids lowering it - the two are one rule and drift if they are apart.

        if isinstance(inner, (tl.DecryptedMessageService, tl.DecryptedMessageService8)):
            outcome = await actions_module.handle(self, chat, inner.action)
            self._emit(
                ServiceActionReceived(
                    chat.id, type(inner.action).__name__, inner.action, outcome.applied
                )
            )
            return

        event = MessageReceived(
            chat_id=chat.id,
            random_id=getattr(inner, "random_id", 0),
            seq_no=wrapper.out_seq_no,
            text=getattr(inner, "message", "") or "",
            entities=getattr(inner, "entities", None),
            ttl=getattr(inner, "ttl", 0) or 0,
            media=getattr(inner, "media", None),
            file=getattr(envelope, "file", None),
            reply_to=getattr(inner, "reply_to_random_id", None),
        )
        self._history.setdefault(chat.id, []).append(event)
        self._emit(event)

    # --- helpers --------------------------------------------------------------

    async def _dh_config(self):
        """§1.1, with every §1.2 check applied to what comes back.

        ``random_length=0``: §1.1 warns that "using the server's random sequence in
        its raw form may be unsafe, it must be combined with a client sequence", and
        asking for none removes the question.
        """
        config = await self._client(
            functions.messages.GetDhConfigRequest(version=0, random_length=0)
        )
        p = dh.value_from_bytes(config.p)
        dh.check_config(g=config.g, p=p)
        return config.g, p

    async def _parse_text(self, text: str):
        """``client._parse_message_text`` - the ONE private Telethon attribute this
        package uses (research.md Q3).

        ASYNC, and that is not cosmetic: Telethon's ``_parse_message_text`` is a
        coroutine function. Returning its result unawaited handed the caller a
        coroutine to unpack - `TypeError: cannot unpack non-iterable coroutine
        object` on the first formatted message - while the fallback path returned a
        plain tuple, so the return type depended on which branch ran. Found by the
        live interop run; every unit test missed it because the fake client has no
        such attribute, so only the fallback was ever exercised.

        Documented fallback, named here so it is not re-derived under pressure: if
        it disappears, send the text unparsed and let the caller pass ``entities``.
        tests/unit/test_telethon_canary.py is what fails when that day comes.
        """
        parse = getattr(self._client, "_parse_message_text", None)
        if parse is None:
            return text, None
        try:
            return await parse(text, None)
        except Exception:
            return text, None

    def _require(self, chat_id: int) -> SecretChat:
        chat = self._chats.get(chat_id)
        if chat is None:
            raise KeyError(f"no secret chat {chat_id} in this manager")
        return chat
