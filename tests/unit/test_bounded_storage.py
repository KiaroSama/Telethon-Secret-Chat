"""Bounded custody effects are admitted before external commitments."""

import pytest
import telethon_secret_chat as api

from .fake_client import FakeClient, Wire, establish
from .test_bounded_manager import limits

pytestmark = pytest.mark.timeout(10)


class FailingTransport(FakeClient):
    async def __call__(self, request):
        from telethon.tl import functions

        if isinstance(request, functions.messages.SendEncryptedRequest):
            self.sent.append(request)
            raise OSError("synthetic uncertain transport")
        return await super().__call__(request)


async def test_uncertain_send_keeps_public_original_identity_and_retry(tmp_path):
    wire = Wire()
    original = wire.a
    wire.a = FailingTransport(original.user_id)
    wire.a.peer = wire.b
    wire.b.peer = wire.a
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        with pytest.raises(api.SendPending) as failed:
            await manager.send_message(chat.id, "uncertain fixture")
        record = [r for r in wire.a.sent if type(r).__name__ == "SendEncryptedRequest"][-1]
        assert record.random_id == failed.value.random_id
        with pytest.raises(api.SendPending) as retried:
            await manager.retry_pending(chat.id)
        assert retried.value.random_id == record.random_id
        again = wire.a.sent[-1]
        assert (again.random_id, again.data) == (record.random_id, record.data)
    finally:
        await manager.stop()
        await peer.stop()


async def test_capacity_refuses_upload_before_any_network_effect(tmp_path):
    wire = Wire()
    storage = api.FileStorage(tmp_path / "state.json")
    manager = api.BoundedSecretChatManager(wire.a, storage, limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(manager, peer, wire)
        await manager.stop()
        resumed = api.BoundedSecretChatManager(wire.a, storage, limits=limits(records=2))
        await resumed.start()
        before = len(wire.a.uploaded)
        with pytest.raises(api.ChatPaused):
            await resumed.send_file(chat.id, b"refused file", file_name="file.bin")
        assert len(wire.a.uploaded) == before
        assert not wire.a.discarded
    finally:
        if resumed:
            await resumed.stop()
        await manager.stop()
        await peer.stop()


@pytest.mark.parametrize(
    "setting",
    [
        dict(pause=None, lost=True, since=None, relay=[], ages={}),
        dict(pause=None, lost=False, since=float("nan"), relay=[], ages={}),
    ],
)
def test_invalid_custody_control_refuses_before_subscription(setting):
    store = api.MemoryStorage()
    store.save_setting("bounded_secret_chat_custody_v1", {"1": setting})
    client = FakeClient()
    with pytest.raises(api.ChatPaused):
        api.BoundedSecretChatManager(client, store, limits=limits())
    assert client.handlers == []


@pytest.mark.parametrize(
    "field,value",
    [
        ("records", 0),
        ("records", True),
        ("bytes", float("inf")),
        ("age_seconds", float("nan")),
        ("tasks", 65),
    ],
)
def test_limits_reject_nonfinite_or_invalid_values(field, value):
    with pytest.raises(ValueError):
        limits(**{field: value})


async def test_concurrent_create_reserves_global_slot_before_rpc(tmp_path):
    import asyncio

    class HeldDh(FakeClient):
        def __init__(self):
            super().__init__()
            self.entered, self.release = asyncio.Event(), asyncio.Event()

        async def __call__(self, request):
            if type(request).__name__ == "GetDhConfigRequest":
                self.entered.set()
                await self.release.wait()
            return await super().__call__(request)

    client = HeldDh()
    manager = api.BoundedSecretChatManager(
        client, api.FileStorage(tmp_path / "state.json"), limits=limits(chats=1)
    )
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    task = asyncio.create_task(manager.create(2000))
    try:
        await client.entered.wait()
        with pytest.raises(api.ChatPaused):
            await manager.create(2001)
        client.release.set()
        await task
        assert sum(type(r).__name__ == "RequestEncryptionRequest" for r in client.sent) == 1
    finally:
        client.release.set()
        if not task.done():
            task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await manager.stop()


async def test_deadline_after_send_commit_reports_original_pending_identity(tmp_path):
    import asyncio

    class HeldSend(FakeClient):
        async def __call__(self, request):
            if type(request).__name__ == "SendEncryptedRequest":
                self.sent.append(request)
                raise asyncio.CancelledError
            return await super().__call__(request)

    wire = Wire()
    wire.a = HeldSend(1000)
    wire.a.peer = wire.b
    wire.b.peer = wire.a
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        with pytest.raises(api.SendPending) as failed:
            await manager.send_message(chat.id, "cancelled send fixture")
        sent = wire.a.sent[-1]
        assert failed.value.random_id == sent.random_id
        assert failed.value.cause == "CancelledError"
    finally:
        await manager.stop()
        await peer.stop()


async def test_receive_commit_failure_preserves_existing_relay_and_blocks_resume(tmp_path):
    class FaultStore(api.FileStorage):
        fail = False

        def _write(self):
            if self.fail:
                raise OSError("synthetic ingress commit fault")
            super()._write()

    wire = Wire()
    store = FaultStore(tmp_path / "state.json")
    manager = api.BoundedSecretChatManager(wire.a, store, limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_message(chat.id, "retained prior")
        before = manager.pending_deliveries(chat.id)
        store.fail = True
        await peer.send_message(chat.id, "failed commit")
        store.fail = False
        assert manager.pending_deliveries(chat.id) == before
        assert not manager.pause_status(chat.id).resumable
        with pytest.raises(api.ChatPaused):
            await manager.resume_chat(chat.id)
        assert not wire.a.discarded
    finally:
        store.fail = False
        await manager.stop()
        await peer.stop()


async def test_send_result_storage_fault_preserves_original_pending_wire_record(tmp_path):
    class FaultStore(api.FileStorage):
        fail = False

        def _write(self):
            if self.fail:
                raise OSError("synthetic send-result fault")
            super()._write()

    class FaultResult(FakeClient):
        store = None

        async def __call__(self, request):
            result = await super().__call__(request)
            if type(request).__name__ == "SendEncryptedRequest":
                self.store.fail = True
            return result

    wire = Wire()
    wire.a = FaultResult(1000)
    wire.a.peer = wire.b
    wire.b.peer = wire.a
    store = FaultStore(tmp_path / "state.json")
    wire.a.store = store
    manager = api.BoundedSecretChatManager(wire.a, store, limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(manager, peer, wire)
        with pytest.raises(api.SendPending) as failed:
            await manager.send_message(chat.id, "result fault fixture")
        record = wire.a.sent[-1]
        assert record.random_id == failed.value.random_id
        store.fail = False
        await manager.stop()
        wire.a.store = store
        resumed = api.BoundedSecretChatManager(wire.a, store, limits=limits())
        await resumed.start()
        again = wire.a.sent[-1]
        assert (again.random_id, again.data) == (record.random_id, record.data)
    finally:
        store.fail = False
        if resumed:
            await resumed.stop()
        store.fail = False
        await manager.stop()
        await peer.stop()


@pytest.mark.parametrize(
    "changed",
    [
        {"since": None, "ages": {"out:1": 1.0}},
        {"since": 1.0, "ages": {"out:1": 1.0}},
        {"relay": [{"seq_no": 0, "body": "00"}]},
    ],
)
def test_malformed_identity_age_metadata_is_not_reset_to_now(changed):
    store = api.MemoryStorage()
    control = dict(pause=None, lost=False, since=None, relay=[], ages={})
    control.update(changed)
    store.save_setting("bounded_secret_chat_custody_v1", {"1": control})
    with pytest.raises(api.ChatPaused):
        api.BoundedSecretChatManager(FakeClient(), store, limits=limits())
