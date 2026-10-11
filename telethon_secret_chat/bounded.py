"""Opt-in bounded durable custody and isolated, explicit conversation pauses."""

from __future__ import annotations

import asyncio
import logging
from contextlib import asynccontextmanager

from telethon.tl import types

from .bounded_storage import CustodyStorage, size_of
from .bounded_types import ChatPaused, CustodyLimits, PauseStatus, RelayDelivery
from . import sequence
from .schema import secret_tl as tl
from .chat import ChatState
from .events import DecryptFailed
from .errors import SecretChatError, SendPending
from .manager import SecretChatManager
from .storage import FileStorage, StorageBackend

log = logging.getLogger("telethon_secret_chat")


class BoundedSecretChatManager(SecretChatManager):
    """Durable protocol queues with finite per-chat custody and explicit pauses.

    Raw callback ingress is not coupled to Telethon qts acknowledgment. A refused
    inbound update marks this chat nonresumable; no lossless replay is promised.
    The backend owns encryption, account binding and exclusive process ownership.
    """

    def __init__(self, client, storage: StorageBackend, *, limits: CustodyLimits, admission=None):
        if type(limits) is not CustodyLimits or not isinstance(storage, StorageBackend):
            raise ValueError("bounded custody requires explicit limits and a storage backend")
        if admission is not None and not callable(admission):
            raise ValueError("admission must be a synchronous callable")
        self.limits = limits
        self._custody = CustodyStorage(storage, limits)
        self._admission = admission
        self._active: dict[asyncio.Task, int | None] = {}
        self._media_tasks = 0
        self._media_bytes = 0
        self._creating_slots = 0
        self._reservations: dict[int, dict[asyncio.Task, tuple[int, int]]] = {}
        self._failed_pauses: set[int] = set()
        from .bounded_transport import AdmittedClient

        super().__init__(AdmittedClient(self, client), self._custody, history_limit=0)
        if isinstance(storage, FileStorage):
            self._default_save_folder = storage.path.parent / "saved-secret-chats"

    async def receive_file(self, message, *, max_bytes):
        """Return verified decoded media in capped RAM; never write a content file."""
        from .bounded_media import receive

        return await receive(self, message, max_bytes)

    def pause_chat(self, chat_id):
        """Stop future effects locally; preserve keys/queues and never remote-discard."""
        self._require(chat_id)
        self._pause(chat_id, "operator")

    def _pause(self, chat_id, reason, lost=False):
        try:
            self._custody.pause(chat_id, reason, lost)
        except Exception:
            self._failed_pauses.add(chat_id)
            control = self._custody.control(chat_id)
            control["pause"], control["lost"] = "storage", True
        log.warning("bounded conversation %s paused (%s)", chat_id, reason)
        self._emit(DecryptFailed(chat_id, "bounded conversation paused; inspect pause_status"))

    def _emit(self, event):
        if (
            len(self._handler_tasks) + len(self._handlers.get(type(event).__name__, []))
            > self.limits.tasks
        ):
            log.warning("bounded callback notification refused; poll durable obligations")
            return
        return super()._emit(event)

    async def _run_handler(self, handler, awaitable):
        try:
            async with asyncio.timeout(self.limits.operation_seconds):
                await super()._run_handler(handler, awaitable)
        except TimeoutError:
            log.warning("bounded callback deadline reached; obligation remains pending")

    def _close_local(self, chat, reason, history_deleted=False):
        # Fatal protocol input stops participation without deleting bounded custody.
        self._pause(chat.id, "ingress", lost=True)

    def _remove_chat(self, chat):
        self._pause(chat.id, "operator", lost=True)

    async def close(self, chat_id, reason="closed by this application"):
        self._require(chat_id)
        self._pause(chat_id, "operator", lost=True)

    async def delete_secret_chat(self, chat_id):
        raise ChatPaused(chat_id=chat_id)

    async def delete_secret_chat_both_sides(self, chat_id):
        raise ChatPaused(chat_id=chat_id)

    async def forget(self, chat_id):
        raise ChatPaused(chat_id=chat_id)

    async def _discard_remote(self, chat_id, delete_history=False):
        self._pause(chat_id, "ingress", lost=True)
        return False

    def pause_status(self, chat_id):
        """Content-free custody usage and continuation eligibility; no read receipt."""
        self._require(chat_id)
        control = self._custody.control(chat_id)
        count, amount = self._custody.usage(chat_id)
        return PauseStatus(
            chat_id, bool(control["pause"]), not control["lost"], control["pause"], count, amount
        )

    def check_custody(self):
        """Check age/capacity now; pause only affected conversations, never evict."""
        for chat_id in self._chats:
            if self._custody.control(chat_id)["pause"]:
                continue
            try:
                self._custody.check(chat_id)
            except ChatPaused:
                self._pause(chat_id, "age")

    async def resume_chat(self, chat_id):
        """Explicitly continue only intact custody; never recover missed ingress."""
        async with self._chat_lock(chat_id):
            self._require(chat_id)
            control = self._custody.control(chat_id)
            if control["lost"] or chat_id in self._failed_pauses:
                raise ChatPaused(chat_id=chat_id)
            self._custody.check(chat_id)
            with self._custody.transaction():
                control["pause"] = None
            chat = self._require(chat_id)
            if chat.state in (ChatState.READY, ChatState.REKEYING):
                await self._drain_deliveries(chat)
                await self.retry_pending(chat_id)
                await self._request_due_resend(chat)

    def _admit(self, chat_id, operation, *, records=0, amount=0):
        if chat_id not in self._chats and str(chat_id) not in self._custody.controls:
            raise ChatPaused(chat_id=chat_id)
        control = self._custody.control(chat_id)
        if control["pause"]:
            raise ChatPaused(chat_id=chat_id)
        held = self._reservations.get(chat_id, {})
        task = asyncio.current_task()
        other_records = sum(value[0] for owner, value in held.items() if owner is not task)
        other_bytes = sum(value[1] for owner, value in held.items() if owner is not task)
        try:
            self._custody.check(
                chat_id,
                reserve_records=records + other_records,
                reserve_bytes=amount + other_bytes,
            )
        except ChatPaused:
            self._pause(chat_id, "capacity")
            raise
        if self._admission is not None:
            try:
                allowed = self._admission(chat_id, operation)
                if type(allowed) is not bool or not allowed:
                    raise ChatPaused(chat_id=chat_id)
            except Exception:
                self._pause(chat_id, "admission")
                raise ChatPaused(chat_id=chat_id) from None

    @asynccontextmanager
    async def _work(self, chat_id, operation, *, records=0, amount=0):
        self._admit(chat_id, operation, records=records, amount=amount)
        task = asyncio.current_task()
        assert task is not None
        nested = task in self._active
        if not nested and len(self._active) + self._media_tasks >= self.limits.tasks:
            self._pause(chat_id, "capacity")
            raise ChatPaused(chat_id=chat_id)
        held = self._reservations.setdefault(chat_id, {})
        previous_reservation = held.get(task)
        old_records, old_bytes = previous_reservation or (0, 0)
        held[task] = (max(records, old_records), max(amount, old_bytes))
        if not nested:
            self._active[task] = chat_id
        try:
            async with asyncio.timeout(None if nested else self.limits.operation_seconds):
                yield
        except TimeoutError:
            self._pause(chat_id, "deadline", lost=operation.startswith("receive"))
            raise ChatPaused(chat_id=chat_id) from None
        except SendPending:
            raise  # Original committed random_id remains the caller's reconciliation identity.
        except SecretChatError:
            raise
        except asyncio.CancelledError:
            if operation.startswith("receive"):
                self._pause(chat_id, "ingress", lost=True)
            raise
        except Exception:
            self._pause(chat_id, "storage", lost=operation.startswith("receive"))
            raise ChatPaused(chat_id=chat_id) from None
        finally:
            if previous_reservation is None:
                held.pop(task, None)
            else:
                held[task] = previous_reservation
            if not held:
                self._reservations.pop(chat_id, None)
            if not nested:
                self._active.pop(task, None)

    def _save(self, chat):
        self._custody.control(chat.id)
        return super()._save(chat)

    def _sendable(self, chat_id):
        self._admit(chat_id, "send")
        return super()._sendable(chat_id)

    def _startup_allowed(self, chat):
        return not self._custody.control(chat.id)["pause"]

    async def start(self):
        # Inspect loaded custody before inherited startup can decrypt/retry/deliver.
        for chat_id in self._custody.list():
            self._custody.control(chat_id)
            try:
                self._admit(chat_id, "startup")
            except ChatPaused:
                pass
        await super().start()

    async def _on_update(self, update):
        if not isinstance(update, (types.UpdateEncryption, types.UpdateNewEncryptedMessage)):
            return
        item = getattr(update, "message", getattr(update, "chat", None))
        chat_id = getattr(item, "chat_id", getattr(item, "id", None))
        if type(chat_id) is not int or chat_id <= 0:
            return
        if chat_id not in self._chats and not isinstance(item, types.EncryptedChatRequested):
            if self._creating and isinstance(
                item, (types.EncryptedChat, types.EncryptedChatDiscarded)
            ):
                self._remember_early(item)
            return
        if (
            chat_id not in self._chats
            and len(self._custody.controls) + self._creating_slots >= self.limits.chats
        ):
            self._emit(DecryptFailed(chat_id, "bounded new conversation refused"))
            return
        if isinstance(item, types.EncryptedChatRequested):
            self._custody.control(chat_id)
        try:
            self._admit(chat_id, "receive")
        except ChatPaused:
            self._pause(chat_id, "ingress", lost=True)
            return
        try:
            await super()._on_update(update)
        except asyncio.CancelledError:
            self._pause(chat_id, "ingress", lost=True)
            raise

    async def _receive_refused(self, chat, failure):
        if isinstance(failure, ChatPaused):
            self._pause(chat.id, "ingress", lost=True)

    async def _on_encrypted_message(self, message):
        async with self._chat_lock(message.chat_id):
            amount = len(message.bytes)
            if amount > self.limits.frame_bytes:
                self._pause(message.chat_id, "ingress", lost=True)
                return
            try:
                # Hex body/envelope and eventual relay coexist; reserve before decrypt.
                async with self._work(
                    message.chat_id, "receive", records=2, amount=amount * 8 + 4096
                ):
                    await super()._on_encrypted_message(message)
            except ChatPaused:
                self._pause(message.chat_id, "ingress", lost=True)

    async def _on_encryption(self, encrypted):
        async with self._chat_lock(encrypted.id):
            try:
                async with self._work(encrypted.id, "receive-handshake", records=4, amount=16384):
                    await super()._on_encryption(encrypted)
            except ChatPaused:
                self._pause(encrypted.id, "ingress", lost=True)
                raise

    async def create(self, user):
        # Reserve globally before the first await; server-assigned chat ID is unknown.
        if (
            len(self._custody.controls) + self._creating_slots >= self.limits.chats
            or len(self._active) + self._media_tasks >= self.limits.tasks
            or self.limits.records < 4
            or self.limits.bytes < 16384
        ):
            raise ChatPaused()
        if self._admission is not None:
            try:
                if self._admission(None, "create") is not True:
                    raise ChatPaused()
            except Exception:
                raise ChatPaused() from None
        task = asyncio.current_task()
        assert task is not None
        self._creating_slots += 1
        self._active[task] = None
        try:
            async with asyncio.timeout(self.limits.operation_seconds):
                return await super().create(user)
        finally:
            self._creating_slots -= 1
            self._active.pop(task, None)

    async def send_message(self, chat_id, text, entities=None, reply_to=None):
        amount = (
            len(text.encode("utf-8")) if isinstance(text, str) else self.limits.frame_bytes + 1
        )
        if amount > self.limits.frame_bytes:
            self._pause(chat_id, "capacity")
            raise ChatPaused(chat_id=chat_id)
        async with self._work(chat_id, "send-message", records=4, amount=amount * 8 + 16384):
            return await super().send_message(chat_id, text, entities, reply_to)

    async def send_file(self, chat_id, source, **kwargs):
        async with self._work(
            chat_id, "upload", records=4, amount=self.limits.frame_bytes * 8 + 16384
        ):
            return await super().send_file(chat_id, source, **kwargs)

    async def forward_file(self, chat_id, source, *, caption="", reply_to=None):
        async with self._work(
            chat_id, "forward", records=4, amount=self.limits.frame_bytes * 8 + 16384
        ):
            return await super().forward_file(chat_id, source, caption=caption, reply_to=reply_to)

    async def accept(self, chat_id):
        async with self._chat_lock(chat_id):
            async with self._work(chat_id, "accept", records=4, amount=16384):
                return await super().accept(chat_id)

    async def _send(self, chat, message, file=None, **kwargs):
        async with self._chat_lock(chat.id):
            amount = size_of(bytes(message))
            if amount > self.limits.frame_bytes:
                self._pause(chat.id, "capacity")
                raise ChatPaused(chat_id=chat.id)
            async with self._work(chat.id, "send", records=1, amount=amount * 4 + 4096):
                return await super()._send(chat, message, file=file, **kwargs)

    async def _transmit(self, chat, item):
        self._admit(chat.id, "rpc")
        try:
            return await super()._transmit(chat, item)
        except asyncio.CancelledError:
            # A timeout/cancel after commitment still exposes the original identity.
            raise SendPending(
                chat_id=chat.id, random_id=item["random_id"], cause="CancelledError"
            ) from None

    async def retry_pending(self, chat_id):
        async with self._chat_lock(chat_id):
            async with self._work(chat_id, "retry"):
                return await super().retry_pending(chat_id)

    def pending_deliveries(self, chat_id):
        """Detached stable obligations; polling neither acknowledges nor marks read."""
        if str(chat_id) not in self._custody.controls:
            self._require(chat_id)
        deliveries = []
        for item in self._custody.control(chat_id)["relay"]:
            wrapper = sequence.unpack(item)
            # The public identity is the wire sequence, exactly as MessageReceived.
            event = self._message_event(chat_id, wrapper)
            deliveries.append(RelayDelivery(chat_id, wrapper.out_seq_no, event))
        return deliveries

    def acknowledge_delivery(self, chat_id, seq_no):
        """Release one consumer-committed obligation, including while paused.

        Persist downstream identity before sending; reconcile unknown cloud results
        before acknowledging. This method sends no RPC or user read receipt.
        """
        if type(seq_no) is not int or seq_no < 0:
            raise ValueError("delivery sequence must be a nonnegative integer")
        with self._custody.transaction():
            control = self._custody.control(chat_id)
            kept = [
                item for item in control["relay"] if sequence.unpack(item).out_seq_no != seq_no
            ]
            if len(kept) == len(control["relay"]):
                raise ValueError("no held relay obligation has this delivery sequence")
            control["relay"] = kept
            self._custody.touched.add(chat_id)

    async def _deliver(self, chat, wrapper, envelope):
        action = getattr(wrapper.message, "action", None)
        if isinstance(
            action,
            (tl.DecryptedMessageActionDeleteMessages, tl.DecryptedMessageActionFlushHistory),
        ):
            self._pause(chat.id, "ingress", lost=True)
            raise ChatPaused(chat_id=chat.id)
        return await super()._deliver(chat, wrapper, envelope)

    async def _rejected(self, chat, item, cause):
        self._pause(chat.id, "ingress", lost=True)
        raise SendPending(chat_id=chat.id, random_id=item["random_id"], cause=cause)

    async def _drain_deliveries(self, chat):
        async with self._chat_lock(chat.id):
            if self._custody.control(chat.id)["pause"] or chat.id in self._delivering:
                return
            self._delivering.add(chat.id)
            try:
                while chat.pending_deliveries and chat.state is not ChatState.CLOSED:
                    self._admit(chat.id, "delivery")
                    item = chat.pending_deliveries[0]
                    wrapper = sequence.unpack(item)
                    ordinary = not isinstance(
                        wrapper.message, (tl.DecryptedMessageService, tl.DecryptedMessageService8)
                    )
                    if ordinary:
                        # Projection and mailbox removal commit together BEFORE callbacks.
                        with self._atomic(chat):
                            control = self._custody.control(chat.id)
                            if not any(
                                record["seq_no"] == item["seq_no"] for record in control["relay"]
                            ):
                                control["relay"].append(item)
                            chat.pending_deliveries.pop(0)
                        await self._deliver(chat, wrapper, None)
                    else:
                        await self._deliver(chat, wrapper, None)
                        if chat.state is not ChatState.CLOSED:
                            with self._atomic(chat):
                                if chat.pending_deliveries and chat.pending_deliveries[0] == item:
                                    chat.pending_deliveries.pop(0)
            finally:
                self._delivering.discard(chat.id)

    async def _request_due_resend(self, chat):
        self._admit(chat.id, "resend")
        return await super()._request_due_resend(chat)

    async def _rekey_if_due(self, chat):
        self._admit(chat.id, "rekey", records=2, amount=16384)
        return await super()._rekey_if_due(chat)

    async def rekey(self, chat_id):
        async with self._work(chat_id, "rekey", records=2, amount=16384):
            return await super().rekey(chat_id)
