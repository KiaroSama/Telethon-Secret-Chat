"""Opt-in bounded transient payloads, with explicit non-resumable refusal."""

from __future__ import annotations

import asyncio
import inspect
import logging
from contextlib import asynccontextmanager
from typing import Any

from telethon.tl import types

from .chat import SecretChat
from .events import DecryptFailed
from .manager import SecretChatManager
from .protected import ProtectedFileStorage, validate_binding
from .transient_storage import TransientStorage
from .transient_work import (
    TransientCleanupIncomplete,
    TransientLimits,
    TransientRefused,
    bounded,
    cancel_owned,
    charged,
)

log = logging.getLogger("telethon_secret_chat")


class TransientSecretChatManager(SecretChatManager):
    """Same-client opt-in. Every detached/reconstructed chat is permanently suspended.

    No lossless replay, transport-qts control, remote abort or RAM erasure promise.
    Only async callbacks are accepted; ordinary and service frames stay in RAM.
    See docs/transient-mode.md for the protected schema and finite budgets.
    """

    def __init__(
        self,
        client: Any,
        storage: ProtectedFileStorage,
        *,
        limits: TransientLimits = TransientLimits(),
    ) -> None:
        if type(storage) is not ProtectedFileStorage or type(limits) is not TransientLimits:
            raise TransientRefused()
        self.limits = limits
        self._transient = TransientStorage(storage, limits)
        super().__init__(client, self._transient, history_limit=0)
        self._owned: dict[asyncio.Task, int | None] = {}
        self._callback_bytes = 0
        self._input_bytes = 0
        self._accepting = True
        self._expiry_handle: asyncio.TimerHandle | None = None
        self._diagnostic_dispatch = False
        self._binding: dict[str, int] | None = None

    def _session_binding(self, user_id: int) -> dict[str, int]:
        try:
            session = self._client.session
            binding = {
                "user_id": user_id,
                "auth_key_id": session.auth_key.key_id,
                "dc_id": session.dc_id,
            }
            validate_binding(binding)
        except Exception:
            raise TransientRefused() from None
        return binding

    def _check_binding(self) -> None:
        if (
            self._binding is None
            or self._session_binding(self._binding["user_id"]) != self._binding
        ):
            raise TransientRefused()

    async def start(self) -> None:
        """Attach without connecting; validate authorization before installing updates."""
        if self._stop_task is not None and not self._stop_task.done():
            raise TransientCleanupIncomplete()
        if self._running:
            if self._stopping:
                raise TransientCleanupIncomplete()
            self._check_binding()
            return
        if any(not task.done() for task in set(self._owned) | set(self._handler_tasks)):
            raise TransientCleanupIncomplete()
        try:
            if not self._client.is_connected():
                raise TransientRefused()
            identity = asyncio.create_task(self._client.get_me())
            self._handler_tasks.add(identity)
            identity.add_done_callback(self._handler_tasks.discard)

            def retrieve_identity(task):
                if not task.cancelled():
                    task.exception()

            identity.add_done_callback(retrieve_identity)
            _, pending = await asyncio.wait({identity}, timeout=self.limits.operation_seconds)
            if pending:
                await cancel_owned({identity}, self.limits.cleanup_seconds)
                raise TransientRefused()
            me = identity.result()
            if me is None or getattr(me, "bot", False):
                raise TransientRefused()
            binding = self._session_binding(me.id)
        except TransientCleanupIncomplete:
            raise
        except asyncio.CancelledError:
            if "identity" in locals():
                await cancel_owned({identity}, self.limits.cleanup_seconds)
            raise
        except Exception:
            raise TransientRefused() from None
        loaded = {}
        protected = self._transient.protected
        for chat_id in protected.list():
            envelope = protected.load(chat_id)
            if envelope is None or envelope["binding"] != binding:
                raise TransientRefused()
            loaded[chat_id] = SecretChat.from_record(
                dict(envelope["chat"], pending_deliveries=[]), stored_id=chat_id
            )
        if len(loaded) > self.limits.chats:
            raise TransientRefused()
        self._binding = self._transient.binding = binding
        self._transient._state = {"chats": {}, "out": {}, "in": {}}
        self._transient.created.clear()
        self._transient.attempts.clear()
        self._chats = loaded
        self._transient.suspended.update(loaded)
        # No replay/handshake/rekey at start: a client can ACK transport updates
        # before the addon sees them; keys/counters alone cannot prove RAM continuity.
        self._transient._refusing = True
        try:
            with self._transient.transaction():
                for chat in loaded.values():
                    self._save(chat)
        finally:
            self._transient._refusing = False
        self._stopping = False
        self._outbound_locks.clear()
        self._client.add_event_handler(self._subscription)
        self._running = True
        for chat_id in loaded:
            self._emit(DecryptFailed(chat_id, "transient state unavailable; chat suspended"))
        log.info("transient secret-chat manager attached")

    def _save(self, chat: SecretChat) -> None:
        self._transient.save(chat.to_record())
        self._arm_expiry()

    def _arm_expiry(self) -> None:
        if self._expiry_handle is not None:
            self._expiry_handle.cancel()
            self._expiry_handle = None
        if self._running and not self._stopping and self._transient.created:
            import time

            due = min(self._transient.created.values()) + self.limits.retention_seconds
            self._expiry_handle = asyncio.get_running_loop().call_later(
                max(0, due - time.monotonic()), self._expire
            )

    def _expire(self) -> None:
        self._expiry_handle = None
        for chat_id in list(self._chats):
            if self._transient.expired(chat_id):
                try:
                    self._suspend(chat_id)
                except TransientRefused:
                    # Already refused in RAM; do not leak exception context to the
                    # event-loop logger or retry a failed store in a hot loop.
                    return
        self._arm_expiry()

    def _suspend(self, chat_id: int) -> None:
        if chat_id not in self._chats or chat_id in self._transient.suspended:
            return
        # In-memory refusal comes first and never rolls back to accepting work if
        # persisting the diagnostic flag fails. Reconstruction always refuses too.
        self._transient.suspended.add(chat_id)
        chat = self._chats.get(chat_id)
        self._transient._refusing = True
        try:
            with self._transient.transaction():
                self._transient.purge(chat_id)
                if chat is not None:
                    chat.pending_deliveries = []
                    self._save(chat)
        except Exception:
            log.error("protected suspension commit failed; processing remains refused")
            raise TransientRefused(chat_id=chat_id) from None
        finally:
            self._transient._refusing = False
            for task, subject in list(self._owned.items()):
                if (
                    subject == chat_id
                    and task is not asyncio.current_task()
                    and task not in self._stop_callers
                    and not task.done()
                ):
                    if not task.cancelling():
                        task.cancel()
        if not self._diagnostic_dispatch:
            self._emit(DecryptFailed(chat_id, "transient processing stopped; chat suspended"))

    def is_suspended(self, chat_id: int) -> bool:
        """Inspect refusal without exposing protected keys or message content."""
        SecretChatManager._require(self, chat_id)
        return chat_id in self._transient.suspended

    def status(self, chat_id: int):
        return SecretChatManager._require(self, chat_id).snapshot()

    def _require(self, chat_id):
        self._check_binding()
        chat = SecretChatManager._require(self, chat_id)
        if self._transient.expired(chat_id):
            self._suspend(chat_id)
        if chat_id in self._transient.suspended:
            raise TransientRefused(chat_id=chat_id)
        return chat

    def _check_current(self, chat) -> None:
        super()._check_current(chat)
        self._require(chat.id)

    def set_accepting_requests(self, enabled: bool) -> None:
        """Stop new create/accept/request intake, not existing conversation traffic."""
        if type(enabled) is not bool:
            raise ValueError("accepting requests must be a boolean")
        self._accepting = enabled

    async def _execute(self, method, args, kwargs, chat_id, size, *, require_known=True):
        if asyncio.current_task() in self._owned:
            async with self._work(chat_id, size):
                return await method(self, *args, **kwargs)
        if len(set(self._owned) | set(self._handler_tasks)) >= self.limits.tasks:
            raise TransientRefused(chat_id=chat_id)
        if not self._running or self._stopping:
            raise TransientRefused(chat_id=chat_id)
        self._check_binding()
        if require_known and chat_id is not None:
            self._require(chat_id)

        async def operation():
            async with self._work(chat_id, size):
                return await method(self, *args, **kwargs)

        child = asyncio.create_task(operation())
        # Reserve ownership before the child first runs. Its work scope will remove
        # it only after actual completion, even if the caller's deadline expires.
        self._handler_tasks.add(child)
        child.add_done_callback(self._handler_tasks.discard)

        def retrieve(task):
            if not task.cancelled():
                task.exception()

        child.add_done_callback(retrieve)
        try:
            _, pending = await asyncio.wait({child}, timeout=self.limits.operation_seconds)
            if pending:
                if chat_id is not None:
                    self._suspend(chat_id)
                else:
                    self._generation += 1
                await cancel_owned({child}, self.limits.cleanup_seconds)
                raise TransientRefused(chat_id=chat_id)
            return child.result()
        except asyncio.CancelledError:
            if chat_id is not None:
                self._suspend(chat_id)
            else:
                self._generation += 1
            await cancel_owned({child}, self.limits.cleanup_seconds)
            raise

    @asynccontextmanager
    async def _work(self, chat_id: int | None, size: int = 0):
        task = asyncio.current_task()
        assert task is not None
        if task in self._owned:
            self._check_binding()
            if chat_id is not None:
                self._require(chat_id)
            if (
                charged(self._transient._state) * 4
                + self._callback_bytes
                + self._input_bytes
                + size
                > self.limits.bytes
            ):
                raise TransientRefused(chat_id=chat_id)
            self._input_bytes += size
            self._transient.external_bytes = self._callback_bytes + self._input_bytes
            try:
                yield
            finally:
                self._input_bytes -= size
                self._transient.external_bytes = self._callback_bytes + self._input_bytes
            return
        if not self._running or self._stopping:
            raise TransientRefused(chat_id=chat_id)
        self._check_binding()
        if (
            len((set(self._owned) | set(self._handler_tasks)) - {task}) >= self.limits.tasks
            or charged(self._transient._state) * 4
            + self._callback_bytes
            + self._input_bytes
            + size
            > self.limits.bytes
        ):
            if chat_id is not None:
                self._suspend(chat_id)
            raise TransientRefused(chat_id=chat_id)
        self._owned[task] = chat_id
        self._input_bytes += size
        self._transient.external_bytes = self._callback_bytes + self._input_bytes
        try:
            async with asyncio.timeout(self.limits.operation_seconds):
                yield
        except asyncio.CancelledError:
            if chat_id is not None:
                self._suspend(chat_id)
            raise
        except Exception:
            if chat_id is not None:
                self._suspend(chat_id)
            # Existing transport/media errors may include inputs. Never expose their
            # text or chained frames across this stricter public boundary.
            raise TransientRefused(chat_id=chat_id) from None
        finally:
            self._owned.pop(task, None)
            self._input_bytes -= size
            self._transient.external_bytes = self._callback_bytes + self._input_bytes

    async def _on_update(self, update) -> None:
        if not isinstance(update, (types.UpdateEncryption, types.UpdateNewEncryptedMessage)):
            return
        if not self._running or self._stopping:
            return
        if (
            isinstance(update, types.UpdateNewEncryptedMessage)
            and update.message.chat_id not in self._chats
        ):
            return  # Unknown ciphertext cannot allocate unbounded per-chat locks.
        item = getattr(update, "message", getattr(update, "chat", None))
        chat_id = getattr(item, "chat_id", getattr(item, "id", None))
        if isinstance(update, types.UpdateEncryption) and chat_id not in self._chats:
            if not isinstance(update.chat, types.EncryptedChatRequested) and not self._creating:
                return
        if chat_id in self._transient.suspended:
            return
        try:
            size = (
                len(update.message.bytes) * 4
                if isinstance(update, types.UpdateNewEncryptedMessage)
                else 4096
            )
            if isinstance(update, types.UpdateNewEncryptedMessage):
                if len(update.message.bytes) > self.limits.frame_bytes:
                    raise TransientRefused(chat_id=chat_id)
            await self._execute(
                SecretChatManager._on_update, (update,), {}, chat_id, size, require_known=False
            )
        except asyncio.CancelledError:
            raise
        except Exception:
            if chat_id is not None:
                self._suspend(chat_id)
            log.warning("transient update refused; no payload or exception text logged")
        finally:
            if chat_id is not None and chat_id not in self._chats:
                self._locks.pop(chat_id, None)

    def _remember_early(self, encrypted):
        if (
            encrypted.id not in self._early_encryption
            and len(self._early_encryption) >= self.limits.chats
        ):
            return
        super()._remember_early(encrypted)

    async def _on_encryption(self, encrypted) -> None:
        if encrypted.id in self._transient.suspended:
            return
        if isinstance(encrypted, types.EncryptedChatRequested) and not self._accepting:
            return
        if isinstance(encrypted, types.EncryptedChatDiscarded):
            self._suspend(encrypted.id)
            return
        if encrypted.id not in self._chats and len(self._chats) >= self.limits.chats:
            raise TransientRefused()
        await super()._on_encryption(encrypted)

    async def create(self, user):
        if not self._accepting or len(self._chats) >= self.limits.chats:
            raise TransientRefused()
        return await self._execute(SecretChatManager.create, (user,), {}, None, 4096)

    async def accept(self, chat_id):
        if not self._accepting:
            raise TransientRefused(chat_id=chat_id)
        return await self._execute(SecretChatManager.accept, (chat_id,), {}, chat_id, 4096)

    async def _deliver(self, chat, wrapper, envelope):
        from .schema import secret_tl as tl

        ordinary = not isinstance(
            wrapper.message, (tl.DecryptedMessageService, tl.DecryptedMessageService8)
        )
        if ordinary and not self._handlers.get("MessageReceived"):
            self._suspend(chat.id)
            raise TransientRefused(chat_id=chat.id)
        return await super()._deliver(chat, wrapper, envelope)

    async def _send(self, chat, message, file=None, **kwargs):
        if len(bytes(message)) > self.limits.frame_bytes - 128:
            raise TransientRefused(chat_id=chat.id)
        return await super()._send(chat, message, file, **kwargs)

    async def _transmit(self, chat, item):
        self._check_current(chat)
        self._transient.attempt(chat.id, item["seq_no"])
        return await super()._transmit(chat, item)

    async def _transmit_or_pending(self, chat, item):
        from .errors import SendPending

        try:
            return await super()._transmit_or_pending(chat, item)
        except SendPending:
            # A durable-retry promise is false here. Refuse the chat rather than
            # encourage a second send or imply that a crash can replay this slot.
            self._suspend(chat.id)
            raise TransientRefused(chat_id=chat.id) from None

    async def _rejected(self, chat, item, cause):
        # Never rewrite a failed ordinary frame as an automatic deletion.
        self._suspend(chat.id)
        raise TransientRefused(chat_id=chat.id)

    def _close_local(self, chat, reason, history_deleted=False):
        self._suspend(chat.id)

    async def close(self, chat_id, reason="closed by this application"):
        self._suspend(chat_id)

    async def _discard_remote(self, chat_id, delete_history=False) -> bool:
        self._suspend(chat_id)
        return False

    def _remove_chat(self, chat):
        self._suspend(chat.id)

    def _forget_locally(self, chat, random_ids):
        # Peer deletion cannot silently rewrite exact retained ordinary slots.
        self._suspend(chat.id)

    def _forget_everything_locally(self, chat):
        self._suspend(chat.id)

    async def _discard_lost_request(self, chat_id):
        self._emit(DecryptFailed(chat_id, "unknown transient request refused; not discarded"))

    def on(self, event: str, handler) -> None:
        if not inspect.iscoroutinefunction(handler):
            raise ValueError("transient callbacks must be async functions")
        if sum(len(items) for items in self._handlers.values()) >= self.limits.tasks:
            raise TransientRefused()
        super().on(event, handler)

    def _emit(self, event) -> None:
        if isinstance(event, DecryptFailed) and event.chat_id in self._chats:
            if event.chat_id not in self._transient.suspended:
                self._suspend(event.chat_id)
                return
        if self._stopping:
            return
        handlers = tuple(self._handlers.get(type(event).__name__, ()))
        if not handlers:
            return
        # Charge one shared event for each outstanding callback. No dataclass repr.
        try:
            size = charged(vars(event))
        except TransientRefused:
            size = self.limits.bytes + 1
        if (
            len(set(self._handler_tasks) | set(self._owned)) + len(handlers) > self.limits.tasks
            or charged(self._transient._state) * 4
            + self._input_bytes
            + self._callback_bytes
            + size * len(handlers)
            > self.limits.bytes
        ):
            # No recursive failure-event dispatch when the callback budget is full.
            self._diagnostic_dispatch = True
            try:
                self._suspend(event.chat_id)
            finally:
                self._diagnostic_dispatch = False
            log.warning("transient callback budget exhausted; chat suspended")
            return
        for handler in handlers:
            self._callback_bytes += size
            self._transient.external_bytes = self._callback_bytes + self._input_bytes
            task = asyncio.create_task(self._callback(handler, event))
            self._handler_tasks.add(task)

            def done(task, size=size):
                self._handler_tasks.discard(task)
                self._callback_bytes -= size
                self._transient.external_bytes = self._callback_bytes + self._input_bytes
                if not task.cancelled():
                    task.exception()

            task.add_done_callback(done)

    async def _callback(self, handler, event) -> None:
        try:
            diagnostic = isinstance(event, DecryptFailed)
            async with self._work(None if diagnostic else event.chat_id):
                if not diagnostic:
                    self._require(event.chat_id)
                await handler(event)
        except asyncio.CancelledError:
            raise
        except Exception:
            self._suspend(event.chat_id)
            log.warning("transient callback failed; no exception text logged")

    async def settle(self) -> None:
        """Await currently owned callbacks without treating them as user reads."""
        tasks = {task for task in self._handler_tasks if task is not asyncio.current_task()}
        if tasks:
            _, pending = await asyncio.wait(tasks, timeout=self.limits.operation_seconds)
            if pending:
                await cancel_owned(pending, self.limits.cleanup_seconds)
                raise TransientRefused()

    async def _finish_stop(self) -> None:
        self._client.remove_event_handler(self._subscription)
        if self._expiry_handle is not None:
            self._expiry_handle.cancel()
            self._expiry_handle = None
        callers = self._stop_callers
        tasks = (set(self._owned) | set(self._handler_tasks)) - callers
        self._draining_handlers.update(tasks)
        try:
            await cancel_owned(tasks, self.limits.cleanup_seconds)
        finally:
            self._draining_handlers.difference_update(tasks)
        for chat_id in list(self._chats):
            self._suspend(chat_id)
        self._running = False
        self._early_encryption.clear()
        self._generation += 1
        log.info("transient secret-chat manager detached; protected keys retained")

    async def send_file(self, chat_id, source, **kwargs) -> int:
        from .media import Source

        try:
            origin = Source(source, kwargs.get("file_name"))
            with origin.open() as (_, size):
                if size > self.limits.bytes // 4:
                    raise TransientRefused(chat_id=chat_id)
            reservation = size * 4 + charged(kwargs)
        except Exception:
            raise TransientRefused(chat_id=chat_id) from None
        return await self._execute(
            SecretChatManager.send_file, (chat_id, source), kwargs, chat_id, reservation
        )

    async def receive_file(self, message) -> bytes:
        """Download and decrypt in bounded RAM, without a disk intermediate."""
        from .transient_media import receive_file

        from . import files

        try:
            media, _ = files._file_of(message)
            chat_id = message.chat_id
            reservation = media.size * 4 + 1024
        except Exception:
            raise TransientRefused() from None
        return await self._execute(receive_file, (message,), {}, chat_id, reservation)

    def _persistence_refused(self, *args, **kwargs):
        raise TransientRefused()

    async def _async_persistence_refused(self, *args, **kwargs):
        raise TransientRefused()

    # The existing signatures/order/crypto remain the implementation; one bounded
    # wrapper owns admission at the public seam rather than copying protocol code.
    send_message = bounded(SecretChatManager.send_message)
    rekey = bounded(SecretChatManager.rekey)
    retry_pending = bounded(SecretChatManager.retry_pending)
    set_ttl = bounded(SecretChatManager.set_ttl)
    mark_read = bounded(SecretChatManager.mark_read)
    screenshot = bounded(SecretChatManager.screenshot)
    set_typing = bounded(SecretChatManager.set_typing)
    forward_file = bounded(SecretChatManager.forward_file)

    start_auto_save_secret_chats = _async_persistence_refused
    stop_auto_save_secret_chats = _async_persistence_refused
    export_saved_messages = _async_persistence_refused
    save_file = _async_persistence_refused
    delete_secret_chat = _async_persistence_refused
    delete_secret_chat_both_sides = _async_persistence_refused
    delete_messages = _async_persistence_refused
    flush_history = _async_persistence_refused
    forget = _async_persistence_refused
    read_saved_messages = _persistence_refused
    delete_saved_messages = _persistence_refused
