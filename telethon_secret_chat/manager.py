"""Durable orchestration of Telethon secret-chat state and ordered delivery.

Storage transactions are synchronous and never span a network await. Per-chat,
reentrant coroutine locks serialize protocol transitions without locking other
chats. Outgoing ciphertext and its sequence are committed together before the
RPC; uncertain sends remain retryable with their original identity and bytes.
The durable mailbox covers receive-to-dispatch. Applications own durable and
idempotent processing of callbacks; scheduling a callback is not its completion.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
import secrets
import time
from contextlib import asynccontextmanager, contextmanager
from copy import deepcopy
from functools import wraps
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

from telethon.extensions import BinaryReader
from telethon.tl import functions, types

from . import actions as actions_module
from . import crypto, dh, files, framing, handshake, rekey as rekey_module, sequence
from .chat import ChatState, SecretChat
from .errors import ChatNotReady, ResendUnsatisfiable, SecretChatError, StorageRequired
from .events import (
    EVENT_TYPES,
    ChatClosedEvent,
    ChatReady,
    ChatRequested,
    DecryptFailed,
    MessageAcknowledged,
    MessageReceived,
    ServiceActionReceived,
)
from .schema import secret_tl as tl
from .storage import StorageBackend

__all__ = ["SecretChatManager"]
log = logging.getLogger("telethon_secret_chat")


def serialized(method):
    @wraps(method)
    async def run(self, subject, *args, **kwargs):
        chat_id = (
            subject
            if isinstance(subject, int)
            else getattr(subject, "chat_id", getattr(subject, "id", None))
        )
        async with self._chat_lock(chat_id):
            return await method(self, subject, *args, **kwargs)

    return run


class SecretChatManager:
    def __init__(self, client, storage: Optional[StorageBackend]):
        if storage is None:
            raise StorageRequired()
        self._client = client
        self._storage = storage
        self._chats: Dict[int, SecretChat] = {}
        self._history: Dict[int, List[MessageReceived]] = {}
        self._handlers: Dict[str, List[Callable]] = {}
        self._handler_tasks = set()
        self._locks = {}
        self._lock_owners = {}
        self._inflight = set()
        self._creating = 0
        self._early_encryption = {}
        self._delivering = set()
        self._running = False
        self._stopping = False
        self._subscription = self._on_update

    @asynccontextmanager
    async def _chat_lock(self, chat_id):
        task = asyncio.current_task()
        if self._lock_owners.get(chat_id) is task:
            yield
            return
        lock = self._locks.setdefault(chat_id, asyncio.Lock())
        async with lock:
            self._lock_owners[chat_id] = task
            try:
                yield
            finally:
                self._lock_owners.pop(chat_id, None)

    @contextmanager
    def _atomic(self, chat):
        previous = deepcopy(vars(chat))
        try:
            with self._storage.transaction():
                yield
                self._save(chat)
        except BaseException:
            vars(chat).clear()
            vars(chat).update(previous)
            raise

    async def start(self):
        if self._running:
            return
        loaded = {}
        for chat_id in self._storage.list():
            record = self._storage.load(chat_id)
            if record is not None:
                loaded[chat_id] = SecretChat.from_record(record)
        self._chats = loaded
        self._client.add_event_handler(self._subscription)
        self._running = True
        self._stopping = False
        for chat in self._chats.values():
            if chat.state in (ChatState.REQUESTED, ChatState.PENDING) and not chat.handshake:
                await self.close(chat.id, "legacy pending handshake has no recoverable secret")
            elif chat.state in (ChatState.READY, ChatState.REKEYING):
                try:
                    await self.retry_pending(chat.id)
                    await self._drain_deliveries(chat)
                except Exception:
                    log.warning("chat %s has durable work awaiting retry", chat.id)
        log.info("secret-chat manager started with %s stored chats", len(self._chats))

    async def stop(self):
        if not self._running:
            return
        self._client.remove_event_handler(self._subscription)
        self._stopping = True
        tasks = [task for task in self._handler_tasks if task is not asyncio.current_task()]
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        for chat in self._chats.values():
            async with self._chat_lock(chat.id):
                self._save(chat)
        self._running = False
        self._early_encryption.clear()
        log.info("secret-chat manager stopped")

    def on(self, event: str, handler: Callable):
        event = "ChatClosedEvent" if event == "ChatClosed" else event
        if event not in EVENT_TYPES or not callable(handler):
            raise ValueError("register a known secret-chat event and a callable handler")
        self._handlers.setdefault(event, []).append(handler)

    @staticmethod
    def _handler_name(handler):
        if inspect.isfunction(handler) or inspect.ismethod(handler):
            return handler.__name__
        return type(handler).__name__

    def _emit(self, event: Any):
        for handler in tuple(self._handlers.get(type(event).__name__, [])):
            try:
                result = handler(event)
            except Exception:
                log.error("secret-chat handler %s failed", self._handler_name(handler))
                continue
            if inspect.isawaitable(result):
                task = asyncio.ensure_future(self._run_handler(handler, result))
                self._handler_tasks.add(task)

                def done(task, result=result):
                    self._handler_tasks.discard(task)
                    if inspect.iscoroutine(result):
                        result.close()  # Also close an awaitable cancelled before its wrapper starts.
                    elif asyncio.isfuture(result) and not result.done():
                        result.cancel()

                task.add_done_callback(done)

    async def _run_handler(self, handler, awaitable):
        try:
            await awaitable
        except Exception:
            # Never log exception text, traceback, arguments, or callable repr.
            log.error("secret-chat handler %s failed", self._handler_name(handler))

    def _save(self, chat):
        self._storage.save(chat.to_record())

    async def create(self, user):
        g, p = await self._dh_config()
        secret = handshake.generate_secret()
        peer = await self._client.get_input_entity(user)
        self._creating += 1
        try:
            result = await self._client(
                functions.messages.RequestEncryptionRequest(
                    user_id=peer,
                    random_id=secrets.randbits(31),
                    g_a=handshake.public_value(g, secret, p).to_bytes(256, "big"),
                )
            )
        finally:
            self._creating -= 1
        chat = SecretChat(
            id=result.id,
            access_hash=result.access_hash,
            peer_user_id=getattr(result, "participant_id", None) or peer.user_id,
            is_outbound=True,
            admin_id=getattr(result, "admin_id", None),
            participant_id=getattr(result, "participant_id", None),
        )
        chat.dh_prime, chat.dh_g = p, g
        chat.handshake = {"secret": secret, "p": p, "g": g}
        self._save(chat)
        self._chats[chat.id] = chat
        early = self._early_encryption.pop(chat.id, None)
        if isinstance(result, types.EncryptedChat):
            await self._on_encryption(result)
        elif early is not None:
            await self._on_encryption(early)
        return chat

    @serialized
    async def accept(self, chat_id):
        chat = self._require(chat_id)
        if chat.state in (ChatState.READY, ChatState.REKEYING):
            return chat
        if chat.state is not ChatState.PENDING:
            chat.require_sendable()
            raise ChatNotReady(chat_id=chat_id, state=chat.state.value)
        pending = chat.handshake
        if "g_a" not in pending:
            raise KeyError(f"no pending request for chat {chat_id}")
        if "secret" not in pending:
            with self._atomic(chat):
                pending["secret"] = handshake.generate_secret()
        key = handshake.shared_key(
            peer_value=pending["g_a"], secret=pending["secret"], p=pending["p"], chat_id=chat_id
        )
        await self._client(
            functions.messages.AcceptEncryptionRequest(
                peer=types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash),
                g_b=handshake.public_value(pending["g"], pending["secret"], pending["p"]).to_bytes(
                    256, "big"
                ),
                key_fingerprint=crypto.key_fingerprint(key),
            )
        )
        with self._atomic(chat):
            chat.adopt_key(key)
            chat.handshake = {}
        self._emit(
            ChatReady(chat.id, chat.peer_user_id, chat.key_fingerprint, chat.initial_key_hash)
        )
        await self._notify_layer(chat)
        return chat

    def _close_local(self, chat, reason):
        with self._atomic(chat):
            # Scrub legacy closed records too, while preserving their first reason.
            reason = chat.closed_reason or reason
            chat.state = ChatState.READY
            chat.close(reason)
            self._storage.delete(chat.id)
        self._history.pop(chat.id, None)
        self._emit(ChatClosedEvent(chat.id, reason))

    @serialized
    async def close(self, chat_id, reason="closed by this application"):
        chat = self._require(chat_id)
        if chat.state is ChatState.CLOSED:
            return
        self._close_local(chat, reason)
        try:
            await self._client(
                functions.messages.DiscardEncryptionRequest(
                    chat_id=chat.id,
                    delete_history=False,
                )
            )
        except Exception:
            log.debug("discardEncryption failed for chat %s; local closure is durable", chat_id)

    def list(self):
        return list(self._chats.values())

    def status(self, chat_id):
        return self._require(chat_id)

    @serialized
    async def send_message(self, chat_id, text, entities=None, reply_to=None):
        chat = self._sendable(chat_id)
        if entities is None and text:
            text, entities = await self._parse_text(text)
        await self._rekey_if_due(chat)
        random_id = secrets.randbits(63)
        await self._send(
            chat,
            tl.DecryptedMessage(
                random_id=random_id,
                ttl=chat.ttl,
                message=text,
                entities=entities or None,
                reply_to_random_id=reply_to,
            ),
        )
        return random_id

    @serialized
    async def rekey(self, chat_id):
        await rekey_module.start(self, self._sendable(chat_id))

    async def _rekey_if_due(self, chat):
        if chat.state is ChatState.READY and rekey_module.should_rekey(chat, time.time()):
            await rekey_module.start(self, chat)

    @serialized
    async def send_file(
        self, chat_id, path, *, caption="", mime_type=None, kind=None, reply_to=None
    ):
        return await files.send(
            self, self._sendable(chat_id), path, caption, mime_type, kind, reply_to
        )

    async def save_file(self, message, path) -> Path:
        return await files.receive(self, message, path)

    @serialized
    async def set_ttl(self, chat_id, seconds):
        chat = self._sendable(chat_id)
        if type(seconds) is not int or not 0 <= seconds < 2**31:
            raise ValueError("TTL must be a nonnegative signed 32-bit integer")
        await self._send_action(
            chat,
            actions_module.set_message_ttl(seconds),
            after_prepare=lambda: setattr(chat, "ttl", seconds),
        )

    async def mark_read(self, chat_id, random_ids):
        await self._send_action(self._sendable(chat_id), actions_module.read_messages(random_ids))

    @serialized
    async def delete_messages(self, chat_id, random_ids):
        chat = self._sendable(chat_id)
        ids = list(random_ids)
        # Remove content BEFORE retrying an uncertain send; privacy cannot wait
        # for the network to succeed. The retained self-delete preserves its slot.
        with self._atomic(chat):
            self._rewrite_retained_as_deletes(chat, set(ids))
        self._remove_history(chat.id, set(ids))
        await self._send_action(chat, actions_module.delete_messages(ids))

    async def screenshot(self, chat_id, random_ids):
        await self._send_action(
            self._sendable(chat_id), actions_module.screenshot_messages(random_ids)
        )

    @serialized
    async def flush_history(self, chat_id):
        chat = self._sendable(chat_id)
        ids = {self._retained_random_id(item) for item in self._storage.retained_out(chat_id)}
        with self._atomic(chat):
            self._rewrite_retained_as_deletes(chat, ids)
        self._history.pop(chat_id, None)
        await self._send_action(chat, actions_module.flush_history())

    async def set_typing(self, chat_id, action=None):
        await self._send_action(self._sendable(chat_id), actions_module.typing(action))

    def _sendable(self, chat_id):
        chat = self._require(chat_id)
        chat.require_sendable()
        return chat

    async def _send_action(self, chat, action, *, after_prepare=None, encryption_key=None):
        await self._send(
            chat,
            tl.DecryptedMessageService(random_id=secrets.randbits(63), action=action),
            after_prepare=after_prepare,
            encryption_key=encryption_key,
        )

    @staticmethod
    def _retained_random_id(item):
        if "random_id" in item:
            return item["random_id"]
        return sequence.unpack(item).message.random_id

    def _rewrite_retained_as_deletes(self, chat, random_ids):
        for item in self._storage.retained_out(chat.id):
            if self._retained_random_id(item) not in random_ids:
                continue
            wrapper = sequence.unpack(item)
            wrapper.message = tl.DecryptedMessageService(
                random_id=wrapper.message.random_id,
                action=actions_module.delete_messages([wrapper.message.random_id]),
            )
            item.update(
                body=bytes(wrapper).hex(),
                frame=crypto.encrypt_frame(chat.key, bytes(wrapper), chat.out_x).hex(),
                method="service",
                file=None,
            )
            self._storage.queue_out(chat.id, item)

    def _remove_history(self, chat_id, random_ids):
        self._history[chat_id] = [
            item for item in self._history.get(chat_id, []) if item.random_id not in random_ids
        ]

    def read_history(self, chat_id, limit=50):
        self._require(chat_id)
        if type(limit) is not int or limit < 0:
            raise ValueError("history limit must be a nonnegative integer")
        return self._history.get(chat_id, [])[-limit:] if limit else []

    @serialized
    async def _send(self, chat, message, file=None, *, after_prepare=None, encryption_key=None):
        chat.require_sendable()
        if self._stopping:
            raise RuntimeError("the secret-chat manager is stopping")
        await self.retry_pending(chat.id)
        with self._atomic(chat):
            wrapper = framing.wrap(
                message,
                layer=framing.outgoing_layer(chat.layer),
                in_seq_no=framing.transform_in_seq_no(chat.in_seq_no, chat.is_outbound),
                out_seq_no=framing.transform_out_seq_no(chat.out_seq_no, chat.is_outbound),
            )
            body = bytes(wrapper)
            item = {
                "seq_no": wrapper.out_seq_no,
                "body": body.hex(),
                "frame": crypto.encrypt_frame(encryption_key or chat.key, body, chat.out_x).hex(),
                "random_id": message.random_id,
                "pending": True,
                "method": (
                    "file"
                    if file is not None
                    else (
                        "service"
                        if isinstance(
                            message, (tl.DecryptedMessageService, tl.DecryptedMessageService8)
                        )
                        else "message"
                    )
                ),
                "file": bytes(file).hex() if file is not None else None,
            }
            chat.out_seq_no += 1
            chat.messages_since_rekey += 1
            self._storage.queue_out(chat.id, item)
            if after_prepare is not None:
                after_prepare()
        await self._transmit(chat, item)

    @serialized
    async def retry_pending(self, chat_id):
        chat = self._sendable(chat_id)
        for item in self._storage.retained_out(chat_id):
            if item.get("pending") and (chat_id, item["seq_no"]) not in self._inflight:
                await self._transmit(chat, item)

    async def _transmit(self, chat, item):
        peer = types.InputEncryptedChat(chat_id=chat.id, access_hash=chat.access_hash)
        if "frame" not in item:
            # Old stores never recorded the RPC identity, cipher, or file handle.
            # Guessing would turn an acknowledged ciphertext into a different send.
            failure = ResendUnsatisfiable(
                chat_id=chat.id,
                requested=(item["seq_no"], item["seq_no"]),
                retained_from=item["seq_no"],
            )
            failure.fatal = True
            await self.close(chat.id, "legacy retained message has no original wire record")
            raise failure
        arguments = dict(peer=peer, random_id=item["random_id"], data=bytes.fromhex(item["frame"]))
        if item.get("method") == "file":
            with BinaryReader(bytes.fromhex(item["file"])) as reader:
                arguments["file"] = reader.tgread_object()
            request = functions.messages.SendEncryptedFileRequest(**arguments)
        elif item.get("method") == "service":
            request = functions.messages.SendEncryptedServiceRequest(**arguments)
        else:
            request = functions.messages.SendEncryptedRequest(**arguments)
        identity = (chat.id, item["seq_no"])
        self._inflight.add(identity)
        try:
            result = await self._client(request)
            with self._storage.transaction():
                # A nested receive may have acknowledged/deleted this record already.
                retained = self._storage.retained_out(chat.id)
                if any(
                    record["seq_no"] == item["seq_no"] and record.get("body") == item.get("body")
                    for record in retained
                ):
                    item = dict(item, pending=False)
                    attached = getattr(result, "file", None)
                    if isinstance(attached, types.EncryptedFile):
                        item["file"] = bytes(
                            types.InputEncryptedFile(
                                id=attached.id,
                                access_hash=attached.access_hash,
                            )
                        ).hex()
                    self._storage.queue_out(chat.id, item)
        finally:
            self._inflight.discard(identity)

    async def _resend_retained(self, chat, retained):
        await self._transmit(chat, retained)

    async def _notify_layer(self, chat):
        await self._send_action(chat, actions_module.notify_layer(framing.MAX_LAYER))

    async def _on_update(self, update):
        try:
            if isinstance(update, types.UpdateEncryption):
                await self._on_encryption(update.chat)
            elif isinstance(update, types.UpdateNewEncryptedMessage):
                await self._on_encrypted_message(update.message)
        except SecretChatError as failure:
            self._emit(DecryptFailed(failure.chat_id or 0, getattr(failure, "reason", "refused")))
        except Exception:
            log.error("secret-chat update failed; no received data or exception text is logged")
            item = getattr(update, "message", getattr(update, "chat", None))
            chat_id = getattr(item, "chat_id", getattr(item, "id", 0))
            self._emit(
                DecryptFailed(chat_id, "update processing failed; durable work may need retry")
            )

    @serialized
    async def _on_encryption(self, encrypted):
        if isinstance(encrypted, types.EncryptedChatRequested):
            if encrypted.id in self._chats:
                return  # Duplicate requests cannot replace a keyed chat or a tombstone.
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
            chat.handshake = {"g_a": g_a, "p": p, "g": g}
            self._save(chat)
            self._chats[chat.id] = chat
            self._emit(ChatRequested(chat.id, encrypted.admin_id))
        elif isinstance(encrypted, types.EncryptedChat):
            chat = self._chats.get(encrypted.id)
            if chat is None:
                if self._creating and len(self._early_encryption) < 100:
                    self._early_encryption[encrypted.id] = encrypted
                return
            if chat.state is ChatState.CLOSED or chat.key is not None:
                return
            pending = chat.handshake
            if "secret" not in pending:
                await self.close(chat.id, "the initial exchange cannot be recovered")
                return
            try:
                key = handshake.shared_key(
                    peer_value=dh.value_from_bytes(encrypted.g_a_or_b),
                    secret=pending["secret"],
                    p=pending["p"],
                    chat_id=chat.id,
                )
                handshake.verify_fingerprint(
                    chat_id=chat.id, key=key, claimed=encrypted.key_fingerprint
                )
            except SecretChatError as failure:
                await self.close(chat.id, failure.reason)
                raise
            with self._atomic(chat):
                chat.adopt_key(key)
                chat.handshake = {}
            self._emit(
                ChatReady(chat.id, chat.peer_user_id, chat.key_fingerprint, chat.initial_key_hash)
            )
            await self._notify_layer(chat)
        elif isinstance(encrypted, types.EncryptedChatDiscarded):
            chat = self._chats.get(encrypted.id)
            if chat is not None and chat.state is not ChatState.CLOSED:
                self._close_local(chat, "the peer discarded the chat")

    @serialized
    async def _on_encrypted_message(self, message):
        chat = self._chats.get(message.chat_id)
        if chat is None or chat.key is None or chat.state is ChatState.CLOSED:
            self._emit(DecryptFailed(message.chat_id, "no usable key is held for this chat"))
            return
        answer, acknowledgements, switched = [], [], False
        try:
            named = rekey_module.select_key(
                chat, int.from_bytes(message.bytes[:8], "little", signed=True)
            )
            if named is None:
                self._emit(DecryptFailed(chat.id, "no key this chat holds matches the frame"))
                return
            # A fingerprint is routing metadata, never proof that the sender knows a key.
            body = crypto.decrypt_frame(named, message.bytes, chat.in_x, chat_id=chat.id)
            wrapper = framing.unwrap(body, chat_id=chat.id)
            with self._atomic(chat):
                if not sequence.preflight(chat, wrapper, self._storage):
                    return
                inner = wrapper.message
                action = getattr(inner, "action", None)
                if isinstance(action, tl.DecryptedMessageActionResend):
                    answer = sequence.answer_resend(
                        chat, self._storage, action.start_seq_no, action.end_seq_no
                    )
                    inner.action = tl.DecryptedMessageActionNoop()
                previous_ack = chat.peer_in_seq_no
                accepted = sequence.accept(chat, wrapper, self._storage, envelope=message)
                if named is chat.pending_key:
                    rekey_module.adopt_new_key(chat, named)
                    chat.state = ChatState.READY
                    chat.new_key_confirmed = True
                    switched = True
                elif named is chat.key and chat.previous_key is not None:
                    chat.new_key_confirmed = True
                chat.messages_since_rekey += 1
                if chat.peer_in_seq_no > previous_ack:
                    ceiling = framing.transform_out_seq_no(
                        chat.peer_in_seq_no - 1, chat.is_outbound
                    )
                    for item in self._storage.retained_out(chat.id):
                        if item["seq_no"] <= ceiling:
                            acknowledgements.append(
                                MessageAcknowledged(
                                    chat.id,
                                    item["seq_no"],
                                    [self._retained_random_id(item)],
                                )
                            )
                sequence.forget_acknowledged(chat, self._storage, chat.peer_in_seq_no)
                chat.pending_deliveries.extend(sequence.pack(item) for item in accepted.ready)
        except SecretChatError as failure:
            if getattr(failure, "fatal", False):
                await self.close(chat.id, failure.reason)
            self._emit(DecryptFailed(chat.id, getattr(failure, "reason", "refused")))
            return
        for event in acknowledgements:
            self._emit(event)
        for retained in answer:
            await self._resend_retained(chat, retained)
        await self._drain_deliveries(chat)
        if chat.state is ChatState.CLOSED:
            return
        with self._atomic(chat):
            rekey_module.retire_previous_key_if_settled(chat)
        if accepted.resend is not None:
            await self._send_action(chat, actions_module.resend(*accepted.resend))
        if switched:
            await self._send_action(chat, tl.DecryptedMessageActionNoop())

    @serialized
    async def _drain_deliveries(self, chat):
        if chat.id in self._delivering:
            return
        self._delivering.add(chat.id)
        try:
            while chat.pending_deliveries and chat.state is not ChatState.CLOSED:
                item = chat.pending_deliveries[0]
                await self._deliver(chat, sequence.unpack(item), None)
                with self._atomic(chat):
                    if chat.pending_deliveries and chat.pending_deliveries[0] == item:
                        chat.pending_deliveries.pop(0)
        finally:
            self._delivering.discard(chat.id)

    async def _deliver(self, chat, wrapper, envelope):
        inner = wrapper.message
        if isinstance(inner, (tl.DecryptedMessageService, tl.DecryptedMessageService8)):
            outcome = await actions_module.handle(self, chat, inner.action)
            self._emit(
                ServiceActionReceived(
                    chat.id, type(inner.action).__name__, inner.action, outcome.applied
                )
            )
            return
        attached = getattr(wrapper, "_tsc_file", None)
        if attached:
            with BinaryReader(bytes.fromhex(attached)) as reader:
                attached = reader.tgread_object()
        event = MessageReceived(
            chat_id=chat.id,
            random_id=getattr(inner, "random_id", 0),
            seq_no=wrapper.out_seq_no,
            text=getattr(inner, "message", "") or "",
            entities=getattr(inner, "entities", None),
            ttl=getattr(inner, "ttl", 0) or 0,
            media=getattr(inner, "media", None),
            file=attached,
            reply_to=getattr(inner, "reply_to_random_id", None),
        )
        history = self._history.setdefault(chat.id, [])
        if not any(previous.random_id == event.random_id for previous in history):
            history.append(event)
        self._emit(event)

    async def _dh_config(self):
        config = await self._client(
            functions.messages.GetDhConfigRequest(version=0, random_length=0)
        )
        p = dh.value_from_bytes(config.p)
        dh.check_config(g=config.g, p=p)
        return config.g, p

    async def _parse_text(self, text):
        parse = getattr(self._client, "_parse_message_text", None)
        if parse is None:
            return text, None
        try:
            return await parse(text, None)
        except Exception:
            log.debug("text parser failed; using caller text without generated entities")
            return text, None

    def _require(self, chat_id):
        chat = self._chats.get(chat_id)
        if chat is None:
            raise KeyError(f"no secret chat {chat_id} in this manager")
        return chat
