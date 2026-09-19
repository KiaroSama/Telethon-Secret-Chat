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

import logging
import secrets
from typing import Any, Callable, Dict, List, Optional

from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, dh, framing, handshake, sequence
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
        for handler in self._handlers.get(type(event).__name__, []):
            handler(event)

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

    async def send_message(self, chat_id: int, text: str, entities=None) -> int:
        """contracts §2. Resolves when TELEGRAM accepts the ciphertext, not when the
        peer acknowledges - acknowledgement arrives as ``MessageAcknowledged``."""
        chat = self._require(chat_id)
        chat.require_sendable()
        random_id = secrets.randbits(63)
        if entities is None and text:
            text, entities = self._parse_text(text)
        message = tl.DecryptedMessage(
            random_id=random_id,
            ttl=chat.ttl,
            message=text,
            entities=entities or None,
        )
        await self._send(chat, message)
        return random_id

    def read_history(self, chat_id: int, limit: int = 50) -> List[MessageReceived]:
        self._require(chat_id)
        return self._history.get(chat_id, [])[-limit:]

    # --- sending --------------------------------------------------------------

    async def _send(self, chat: SecretChat, message) -> None:
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
        await self._client(
            functions.messages.SendEncryptedRequest(
                peer=types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash),
                random_id=secrets.randbits(63),
                data=frame,
            )
        )

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
                action=tl.DecryptedMessageActionNotifyLayer(layer=framing.MAX_LAYER),
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
        try:
            body = crypto.decrypt_frame(chat.key, message.bytes, chat.in_x, chat_id=chat.id)
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
        for item in accepted.ready:
            await self._deliver(chat, item, message)
        self._save(chat)
        if accepted.resend is not None:
            # §3.7: ask for exactly the missing span, once per hole.
            await self._send(
                chat,
                tl.DecryptedMessageService(
                    random_id=secrets.randbits(63),
                    action=tl.DecryptedMessageActionResend(
                        start_seq_no=accepted.resend[0], end_seq_no=accepted.resend[1]
                    ),
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

    def _parse_text(self, text: str):
        """``client._parse_message_text`` - the ONE private Telethon attribute this
        package uses (research.md Q3).

        Documented fallback, named here so it is not re-derived under pressure: if
        it disappears, send the text unparsed and let the caller pass ``entities``.
        tests/unit/test_telethon_canary.py is what fails when that day comes.
        """
        parse = getattr(self._client, "_parse_message_text", None)
        if parse is None:
            return text, None
        try:
            return parse(text, None)
        except Exception:
            return text, None

    def _require(self, chat_id: int) -> SecretChat:
        chat = self._chats.get(chat_id)
        if chat is None:
            raise KeyError(f"no secret chat {chat_id} in this manager")
        return chat
