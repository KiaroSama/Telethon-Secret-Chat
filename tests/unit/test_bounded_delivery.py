"""Consumer acknowledgment is separate from scheduling an event handler."""

import pytest
import telethon_secret_chat as api

from .fake_client import Wire, establish
from .test_bounded_manager import limits

pytestmark = pytest.mark.timeout(10)


async def test_callback_failure_leaves_stable_relay_until_explicit_ack_after_restart(tmp_path):
    wire = Wire()
    path = tmp_path / "state.json"
    manager = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(manager, peer, wire)

        def failing(event):
            raise RuntimeError("synthetic consumer failure")

        manager.on("MessageReceived", failing)
        random_id = await peer.send_message(chat.id, "relay fixture")
        pending = manager.pending_deliveries(chat.id)
        assert len(pending) == 1
        assert pending[0].message.random_id == random_id
        assert pending[0].message.text == "relay fixture"
        identity = pending[0].seq_no
        await manager.stop()
        resumed = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
        await resumed.start()
        assert resumed.pending_deliveries(chat.id)[0].seq_no == identity
        resumed.acknowledge_delivery(chat.id, identity)
        assert resumed.pending_deliveries(chat.id) == []
        assert not any("ReadMessages" in type(r).__name__ for r in wire.a.sent)
    finally:
        if resumed is not None:
            await resumed.stop()
        await manager.stop()
        await peer.stop()


async def test_protocol_close_preserves_unacknowledged_consumer_obligation(tmp_path):
    wire = Wire()
    path = tmp_path / "state.json"
    manager = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_message(chat.id, "retained relay")
        held = manager.pending_deliveries(chat.id)[0]
        assert held.seq_no == held.message.seq_no
        await manager.close(chat.id)
        await manager.stop()
        resumed = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
        await resumed.start()
        assert resumed.pending_deliveries(chat.id)[0].message.text == "retained relay"
        resumed.acknowledge_delivery(chat.id, held.seq_no)
        assert resumed.pending_deliveries(chat.id) == []
    finally:
        if resumed:
            await resumed.stop()
        await manager.stop()
        await peer.stop()


async def test_failed_ack_commit_keeps_same_obligation(tmp_path):
    class FaultStore(api.FileStorage):
        fail = False

        def _write(self):
            if self.fail:
                raise OSError("synthetic acknowledgment fault")
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
        await peer.send_message(chat.id, "ack fixture")
        before = manager.pending_deliveries(chat.id)
        store.fail = True
        with pytest.raises(OSError):
            manager.acknowledge_delivery(chat.id, before[0].seq_no)
        store.fail = False
        assert manager.pending_deliveries(chat.id) == before
        manager.acknowledge_delivery(chat.id, before[0].seq_no)
        assert manager.pending_deliveries(chat.id) == []
    finally:
        store.fail = False
        await manager.stop()
        await peer.stop()


async def test_pending_consumer_obligation_does_not_block_protocol_ttl(tmp_path):
    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_message(chat.id, "held relay")
        await peer.set_ttl(chat.id, 31)
        assert manager.status(chat.id).ttl == 31
        assert len(manager.pending_deliveries(chat.id)) == 1
        await manager.send_message(chat.id, "reply fixture")
        assert peer.read_history(chat.id)[-1].ttl == 31
    finally:
        await manager.stop()
        await peer.stop()
