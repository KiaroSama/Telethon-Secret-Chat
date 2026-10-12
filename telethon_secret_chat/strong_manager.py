"""Apply committed ingress through the existing bounded protocol and relay manager."""

import asyncio
from contextlib import contextmanager

from .bounded import BoundedSecretChatManager
from .chat import ChatState
from . import sequence
from .schema import secret_tl as tl
from .ingress import DurableIngress
from .ingress_replay import updates_for, is_secret_update, advance
from .ingress_store import IngressRefused


class StrongSecretChatManager(BoundedSecretChatManager):
    """The caller's pre-configured client; Raw only wakes committed custody."""

    def __init__(self, client, ingress: DurableIngress, *, limits, admission=None):
        if (
            type(ingress) is not DurableIngress
            or getattr(client, "_durable_ingress", None) is not ingress
        ):
            raise IngressRefused()
        if ingress._manager_owner is not None:
            raise IngressRefused()
        self.ingress = ingress
        self._ingress_current: tuple[str, int, int] | None = None
        self._ingress_owner: asyncio.Task | None = None
        self._stage_candidate = None
        self._applied_current = False
        self._ingress_drain_lock = asyncio.Lock()
        self._service_current: dict[asyncio.Task, tuple[int, dict]] = {}
        super().__init__(client, ingress.storage, limits=limits, admission=admission)
        self._default_save_folder = None
        ingress._manager_owner = self

    async def start(self):
        await self.stop_auto_save_secret_chats()
        await super().start()
        await self.drain_ingress()

    async def stop(self):
        self.ingress._stopped = True
        self.ingress._epoch += 1
        owner = self._ingress_owner
        if owner is asyncio.current_task():
            raise IngressRefused()  # Never release ownership from an active receive stage.
        if owner is not None and not owner.done():
            owner.cancel()
            done, _ = await asyncio.wait({owner}, timeout=self.ingress.limits.operation_seconds)
            if owner not in done:
                raise IngressRefused()  # Cancellation-resistant code retains exclusive custody.
        async with asyncio.timeout(self.ingress.limits.operation_seconds):
            async with self._ingress_drain_lock:
                await super().stop()
                if self.ingress._manager_owner is self:
                    self.ingress._manager_owner = None

    async def start_auto_save_secret_chats(self, folder):
        raise IngressRefused()

    def _owns_ingress(self, chat):
        return (
            self._ingress_current is not None
            and self._ingress_owner is asyncio.current_task()
            and self._ingress_current[2] == chat.id
        )

    def _ingress_applied(self, chat):
        # Only exact receive/establishment acceptance calls this hook, not _save.
        if (
            self._ingress_current is not None
            and self._owns_ingress(chat)
            and not self._applied_current
        ):
            identity, cursor, _ = self._ingress_current
            self._stage_candidate = advance(self.ingress, identity, cursor)

    def _ingress_committed(self, chat):
        if self._owns_ingress(chat) and self._stage_candidate is not None:
            self.ingress._state = self._stage_candidate
            self._stage_candidate = None
            self._applied_current = True

    @contextmanager
    def _atomic(self, chat):
        try:
            with super()._atomic(chat):
                yield
        except BaseException:
            if self._owns_ingress(chat):
                self._stage_candidate = None
            raise
        else:
            self._ingress_committed(chat)

    async def _on_update(self, update):
        await self.drain_ingress()

    def _close_local(self, chat, reason, history_deleted=False):
        # A terminal peer update is consumed, but its protected custody is retained.
        with self._storage.transaction():
            self._custody.pause(chat.id, "ingress", lost=True)
            self._ingress_applied(chat)
        self._ingress_committed(chat)

    def _remove_chat(self, chat):
        self._close_local(chat, "terminal peer update")

    def _pause(self, chat_id, reason, lost=False):
        # Refusal/cancel during replay cannot lose the already committed envelope.
        current = self._ingress_current
        if (
            current is not None
            and current[2] == chat_id
            and self._ingress_owner is asyncio.current_task()
        ):
            lost = False
        return super()._pause(chat_id, reason, lost=lost)

    async def resume_chat(self, chat_id):
        await super().resume_chat(chat_id)
        await self.drain_ingress()

    def _service_effect_prepared(self, chat, message=None):
        task = asyncio.current_task()
        current = self._service_current.get(task) if task is not None else None
        if current is not None and current[0] == chat.id:
            item = current[1]
            if message is not None:
                inbound = getattr(sequence.unpack(item).message, "action", None)
                outbound = getattr(message, "action", None)
                responses = {
                    tl.DecryptedMessageActionRequestKey: (
                        tl.DecryptedMessageActionAcceptKey,
                        tl.DecryptedMessageActionAbortKey,
                    ),
                    tl.DecryptedMessageActionAcceptKey: (
                        tl.DecryptedMessageActionCommitKey,
                        tl.DecryptedMessageActionAbortKey,
                    ),
                    tl.DecryptedMessageActionCommitKey: (tl.DecryptedMessageActionNoop,),
                }
                if not isinstance(outbound, responses.get(type(inbound), ())):
                    return
            if chat.pending_deliveries and chat.pending_deliveries[0] == item:
                chat.pending_deliveries.pop(0)

    async def _drain_deliveries(self, chat):
        async with self._chat_lock(chat.id):
            while chat.pending_deliveries and chat.state is not ChatState.CLOSED:
                item = chat.pending_deliveries[0]
                wrapper = sequence.unpack(item)
                if not isinstance(
                    wrapper.message, (tl.DecryptedMessageService, tl.DecryptedMessageService8)
                ):
                    self._admit(chat.id, "delivery")
                    with self._atomic(chat):
                        control = self._custody.control(chat.id)
                        if not any(
                            record["seq_no"] == item["seq_no"] for record in control["relay"]
                        ):
                            control["relay"].append(item)
                        chat.pending_deliveries.pop(0)
                    await self._deliver(chat, wrapper, None)
                    continue
                self._admit(chat.id, "delivery")
                task = asyncio.current_task()
                if task is None:
                    raise IngressRefused()
                self._service_current[task] = chat.id, item
                try:
                    await self._deliver(chat, wrapper, None)
                    with self._atomic(chat):
                        self._service_effect_prepared(chat)
                finally:
                    self._service_current.pop(task, None)
                if self._custody.control(chat.id)["pause"]:
                    return

    async def drain_ingress(self):
        async with asyncio.timeout(self.ingress.limits.operation_seconds):
            await self._drain_ingress()

    async def _drain_ingress(self):
        async with self._ingress_drain_lock:
            self.ingress._ready()
            for batch in self.ingress.pending_batches():
                if not any(
                    item["identity"] == batch.identity for item in self.ingress._state["batches"]
                ):
                    continue  # A preceding completed prefix may have removed this snapshot entry.
                updates = updates_for(batch)
                for index, update in enumerate(updates):
                    if index < batch.cursor or index in batch.applied:
                        continue
                    if not is_secret_update(update):
                        with self.ingress.storage.transaction():
                            candidate = advance(self.ingress, batch.identity, index + 1)
                        self.ingress._state = candidate
                        continue
                    item = getattr(update, "message", getattr(update, "chat", None))
                    chat_id = getattr(item, "chat_id", getattr(item, "id", None))
                    if type(chat_id) is not int or chat_id <= 0:
                        raise IngressRefused()
                    from telethon.tl import types

                    if (
                        chat_id not in self._chats
                        and isinstance(item, types.EncryptedChatDiscarded)
                        and not self._creating
                    ):
                        with self.ingress.storage.transaction():
                            candidate = advance(self.ingress, batch.identity, index + 1)
                        self.ingress._state = candidate
                        continue  # Proven unknown terminal event has no local protocol effects.
                    if chat_id in self._chats and self.pause_status(chat_id).paused:
                        continue
                    self._ingress_current = batch.identity, index + 1, chat_id
                    self._ingress_owner = asyncio.current_task()
                    self._stage_candidate = None
                    self._applied_current = False
                    try:
                        await super()._on_update(update)
                        if not self._applied_current:
                            if chat_id in self._chats and self.pause_status(chat_id).paused:
                                continue
                            raise IngressRefused()
                    finally:
                        self._ingress_current = None
                        self._ingress_owner = None
                        self._stage_candidate = None
                held = next(
                    item
                    for item in self.ingress._state["batches"]
                    if item["identity"] == batch.identity
                )
                if held["cursor"] != len(updates):
                    continue
                with self.ingress.storage.transaction():
                    candidate = dict(self.ingress._state)
                    candidate["batches"] = list(candidate["batches"])
                    while candidate["batches"]:
                        first = candidate["batches"][0]
                        receipt = next(
                            item
                            for item in self.ingress.pending_batches()
                            if item.identity == first["identity"]
                        )
                        if first["cursor"] != len(updates_for(receipt)):
                            break
                        candidate["batches"].pop(0)
                    self.ingress.storage.save_setting("durable_ingress_v1", candidate)
                self.ingress._state = candidate
