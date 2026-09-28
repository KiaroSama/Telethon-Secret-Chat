"""Receive-to-dispatch durability: mailbox, acknowledgements, rekey commits, handlers, gaps."""

import asyncio
from copy import deepcopy

import pytest

from telethon_secret_chat import SecretChatManager, crypto, framing, rekey
from telethon_secret_chat.events import ChatReady
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .dh_material import SAFE_PRIME
from .fake_client import FakeClient, Wire, establish
from .helpers import SEND_TYPES, OTHER, incoming, peer_message, ready_manager


async def test_receive_storage_failure_cannot_advance_sequence_or_emit(monkeypatch):
    manager, chat = ready_manager()
    before = deepcopy(chat.to_record())
    received = []
    manager.on("MessageReceived", received.append)

    def fail():
        raise OSError("synthetic durable receive failure")

    monkeypatch.setattr(manager._storage, "_write", fail)
    with pytest.raises(OSError):
        await manager._on_encrypted_message(incoming(chat, peer_message(0)))
    assert chat.to_record() == before
    assert not received


async def test_delivery_mailbox_restarts_after_dispatch_failure(monkeypatch):
    manager, chat = ready_manager()

    async def fail(*args):
        raise RuntimeError("synthetic interrupted dispatch")

    monkeypatch.setattr(manager, "_deliver", fail)
    with pytest.raises(RuntimeError):
        await manager._on_encrypted_message(incoming(chat, peer_message(0)))
    assert manager._storage.load(chat.id)["in_seq_no"] == 1
    recovered = SecretChatManager(FakeClient(), storage=manager._storage)
    received = []
    recovered.on("MessageReceived", received.append)
    await recovered.start()
    try:
        assert len(received) == 1
        assert not recovered._entity(chat.id).pending_deliveries
    finally:
        await recovered.stop()


async def test_acknowledgement_emitted_once_after_peer_counter_advances():
    manager, chat = ready_manager()
    random_id = await manager.send_message(chat.id, "ack me")
    seen = []
    manager.on("MessageAcknowledged", seen.append)
    packet = incoming(chat, peer_message(0, raw_in=1))
    await manager._on_encrypted_message(packet)
    await manager._on_encrypted_message(packet)
    assert len(seen) == 1
    assert seen[0].random_ids == [random_id]
    assert manager._storage.retained_out(chat.id) == []


async def test_rekey_confirmation_noop_uses_new_key_and_retires_initiator_old_key():
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    try:
        ca, cb = await establish(a, b, wire)
        old = ca.key
        start = len(wire.b.sent)
        await a.rekey(ca.id)
        assert ca.key == cb.key != old
        noops = [
            r
            for r in wire.b.sent[start:]
            if isinstance(r, SEND_TYPES)
            and int.from_bytes(r.data[:8], "little", signed=True) == ca.key_fingerprint
        ]
        assert noops
        wrapper = framing.unwrap(crypto.decrypt_frame(ca.key, noops[-1].data, cb.out_x))
        assert isinstance(wrapper.message.action, tl.DecryptedMessageActionNoop)
        assert ca.previous_key is None
        assert ca.exchange_secret is None
    finally:
        await a.stop()
        await b.stop()


async def test_rekey_preparation_rolls_back_when_storage_rejects(monkeypatch):
    manager, chat = ready_manager()
    chat.dh_prime, chat.dh_g = SAFE_PRIME, 2
    before = deepcopy(chat.to_record())

    def fail():
        raise OSError("synthetic rekey persistence failure")

    monkeypatch.setattr(manager._storage, "_write", fail)
    with pytest.raises(OSError):
        await manager.rekey(chat.id)
    assert chat.to_record() == before
    assert not manager._client.sent


async def test_accepted_rekey_cannot_be_overwritten_by_another_request():
    manager, chat = ready_manager()
    chat.exchange_id, chat.pending_key, chat.rekey_role = 100, OTHER, "accepted"
    before = deepcopy(chat.to_record())
    await rekey.handle(
        manager, chat, tl.DecryptedMessageActionRequestKey(exchange_id=200, g_a=b"bad")
    )
    assert chat.to_record() == before
    assert not manager._client.sent


async def test_async_handler_tasks_are_owned_and_cancelled_on_stop():
    manager = SecretChatManager(FakeClient(), storage=MemoryStorage())
    await manager.start()
    entered, exited = asyncio.Event(), asyncio.Event()

    async def handler(event):
        entered.set()
        try:
            await asyncio.Future()
        finally:
            exited.set()

    manager.on("ChatReady", handler)
    manager._emit(ChatReady(1, 2, 3))
    await asyncio.wait_for(entered.wait(), timeout=5)
    assert manager._handler_tasks
    await manager.stop()
    assert exited.is_set()
    assert not manager._handler_tasks


async def test_handler_cancelled_before_it_starts_does_not_leave_coroutine_open():
    manager = SecretChatManager(FakeClient(), storage=MemoryStorage())
    await manager.start()
    made = []

    async def work():
        await asyncio.sleep(0)

    def handler(event):
        made.append(work())
        return made[-1]

    manager.on("ChatReady", handler)
    manager._emit(ChatReady(1, 2, 3))
    await manager.stop()
    assert made[0].cr_frame is None


def test_full_gap_buffer_can_be_drained_by_the_missing_predecessor():
    from telethon_secret_chat import sequence
    from .helpers import a_chat

    chat, store = a_chat(), MemoryStorage()
    for raw in range(1, sequence.MAX_RESEND_COUNT + 1):
        sequence.accept(chat, peer_message(raw), store)
    result = sequence.accept(chat, peer_message(0), store)
    assert len(result.ready) == sequence.MAX_RESEND_COUNT + 1
    assert not chat.gap_requested


def test_gap_inspection_returns_detached_sorted_data_without_a_write(monkeypatch):
    store = MemoryStorage()
    store.queue_in(7, {"seq_no": 2, "body": "two", "ids": [2]})
    store.queue_in(7, {"seq_no": 1, "body": "one", "ids": [1]})

    def forbidden():
        raise AssertionError("read-only gap inspection attempted to write")

    monkeypatch.setattr(store, "_write", forbidden)
    read = store.peek_in(7)
    assert [item["seq_no"] for item in read] == [1, 2]
    read[0]["ids"].append(999)
    assert store.peek_in(7)[0]["ids"] == [1]


async def test_a_stored_mailbox_is_delivered_at_start_even_when_sends_fail(monkeypatch):
    """A pending send that cannot go out at start() must not hold back stored deliveries."""
    from .helpers import FlakyClient

    manager, chat = ready_manager(FlakyClient())
    manager._client.failing = False

    async def fail(*args):
        raise RuntimeError("synthetic interrupted dispatch")

    monkeypatch.setattr(manager, "_deliver", fail)
    with pytest.raises(RuntimeError):
        await manager._on_encrypted_message(incoming(chat, peer_message(0)))
    manager._storage.queue_out(chat.id, {"seq_no": 1, "random_id": 1, "pending": True})

    recovered = SecretChatManager(FlakyClient(), storage=manager._storage)
    received = []
    recovered.on("MessageReceived", received.append)
    await recovered.start()
    try:
        assert len(received) == 1
    finally:
        await recovered.stop()
