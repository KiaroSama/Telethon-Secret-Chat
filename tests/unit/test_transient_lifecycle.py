"""Finite lifecycle boundaries: no real account, sleep or second connection."""

import asyncio

import pytest

from telethon_secret_chat import (
    MemoryStorage,
    ProtectedFileStorage,
    SecretChatManager,
    TransientLimits,
    TransientRefused,
    TransientSecretChatManager,
)

from .fake_client import Wire, establish
from .test_transient_manager import authorize

pytestmark = pytest.mark.timeout(10)


async def test_reconstruction_refuses_without_erasing_keys_or_sending(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await a.send_message(chat.id, "synthetic crash window")
        before = store.load(chat.id)["chat"]
        await a.stop()
        count = len(wire.a.sent)
        restored = TransientSecretChatManager(wire.a, store)
        await restored.start()
        try:
            assert restored.is_suspended(chat.id)
            with pytest.raises(TransientRefused):
                await restored.send_message(chat.id, "must refuse")
            assert len(wire.a.sent) == count
            after = store.load(chat.id)["chat"]
            assert bool(before["key"] == after["key"])
            assert before["out_seq_no"] == after["out_seq_no"]
            assert not wire.a.discarded
        finally:
            await restored.stop()
    finally:
        await a.stop()
        await b.stop()


async def test_handler_failure_suspends_without_remote_discard(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())

    async def broken(event):
        raise RuntimeError("synthetic callback failure")

    a.on("MessageReceived", broken)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic inbound")
        await a.settle()
        assert a.is_suspended(chat.id)
        assert a.status(chat.id).key_fingerprint is not None
        assert not wire.a.discarded
    finally:
        await a.stop()
        await b.stop()


async def test_disabled_acceptance_keeps_established_chat_serviceable(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        a.set_accepting_requests(False)
        with pytest.raises(TransientRefused):
            await a.create(2000)
        await a.send_message(chat.id, "still live")
        assert not a.is_suspended(chat.id)
    finally:
        await a.stop()
        await b.stop()


@pytest.mark.parametrize(
    "field,value", [("tasks", True), ("operation_seconds", float("inf")), ("attempts", 0)]
)
def test_limits_refuse_unbounded_or_bool_values(field, value):
    with pytest.raises(ValueError):
        TransientLimits(**{field: value})


async def test_shutdown_cancels_waiting_callback_and_keeps_keys(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    entered, released = asyncio.Event(), asyncio.Event()

    async def waiting(event):
        entered.set()
        try:
            await asyncio.Event().wait()
        finally:
            released.set()

    a.on("MessageReceived", waiting)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic callback wait")
        await asyncio.wait_for(entered.wait(), 1)
        await a.stop()
        assert released.is_set()
        assert not a._owned
        assert not a._handler_tasks
        assert a.is_suspended(chat.id)
        assert bool(store.load(chat.id)["chat"]["key"])
    finally:
        await a.stop()
        await b.stop()


async def test_changed_authorization_refuses_before_subscription(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await a.stop()
        before = store.path.read_bytes()
        wire.a.session.auth_key.key_id += 1
        replacement = TransientSecretChatManager(wire.a, store)
        with pytest.raises(TransientRefused):
            await replacement.start()
        assert wire.a.handlers == []
        assert store.path.read_bytes() == before
        assert bool(store.load(chat.id)["chat"]["key"])
    finally:
        await a.stop()
        await b.stop()


async def test_delivery_does_not_send_read_receipt(tmp_path):
    from telethon_secret_chat import sequence
    from telethon_secret_chat.schema import secret_tl as tl

    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    got = []

    async def collect(event):
        got.append(event)

    a.on("MessageReceived", collect)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.set_ttl(chat.id, 30)
        await b.send_message(chat.id, "synthetic timed content")
        await a.settle()
        held = a._storage.retained_out(chat.id)
        assert not any(
            isinstance(
                getattr(sequence.unpack(item).message, "action", None),
                tl.DecryptedMessageActionReadMessages,
            )
            for item in held
        )
        assert got[-1].ttl == 30
        await a.mark_read(chat.id, [got[-1].random_id])
        held = a._storage.retained_out(chat.id)
        assert any(
            isinstance(
                getattr(sequence.unpack(item).message, "action", None),
                tl.DecryptedMessageActionReadMessages,
            )
            for item in held
        )
    finally:
        await a.stop()
        await b.stop()


async def test_malformed_frame_refuses_without_deleting_chat(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        before = store.load(chat.id)["chat"]
        await wire.a.deliver(chat.id, b"bad synthetic frame")
        assert a.is_suspended(chat.id)
        after = store.load(chat.id)["chat"]
        assert bool(before["key"] == after["key"])
        assert before["in_seq_no"] == after["in_seq_no"]
        assert not wire.a.discarded
    finally:
        await a.stop()
        await b.stop()


async def test_gap_payload_only_in_ram_and_expiry_refuses(tmp_path, monkeypatch):
    import time

    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        wire.b.hold = True
        await b.send_message(chat.id, "first synthetic gap")
        await b.send_message(chat.id, "second synthetic gap")
        await wire.a.deliver(*wire.b.held[-1])
        assert len(a._storage.peek_in(chat.id)) == 1
        raw = store.path.read_bytes()
        assert bool(b"second synthetic gap" not in raw)
        before = store.load(chat.id)["chat"]
        future = time.monotonic() + 301
        monkeypatch.setattr(
            "telethon_secret_chat.transient_storage.time",
            type("Clock", (), {"monotonic": staticmethod(lambda: future)}),
        )
        with pytest.raises(TransientRefused):
            await a.send_message(chat.id, "must refuse after expiry")
        assert a.is_suspended(chat.id)
        after = store.load(chat.id)["chat"]
        assert before["in_seq_no"] == after["in_seq_no"]
        assert bool(before["key"] == after["key"])
        assert not wire.a.discarded
    finally:
        await a.stop()
        await b.stop()


async def test_live_rekey_preserves_protected_counters(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        before = store.load(chat.id)["chat"]
        await a.rekey(chat.id)
        after = store.load(chat.id)["chat"]
        assert bool(before["key"] != after["key"])
        assert after["out_seq_no"] > before["out_seq_no"]
        assert after["in_seq_no"] >= before["in_seq_no"]
        assert after["exchange_id"] is None
        assert not a.is_suspended(chat.id)
        assert store.retained_out(chat.id) == []
    finally:
        await a.stop()
        await b.stop()


async def test_cancelled_callback_can_stop_without_shutdown_cycle(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    entered, cleaned = asyncio.Event(), asyncio.Event()

    async def waiting(event):
        entered.set()
        try:
            await asyncio.Event().wait()
        finally:
            await a.stop()
            cleaned.set()

    a.on("MessageReceived", waiting)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic cleanup")
        await asyncio.wait_for(entered.wait(), 1)
        await asyncio.wait_for(a.stop(), 2)
        assert cleaned.is_set()
        assert not a._handler_tasks
        assert not a._owned
    finally:
        await a.stop()
        await b.stop()


def test_real_telethon_authorization_binding_surface_without_connection():
    from telethon import TelegramClient
    from telethon.crypto import AuthKey
    from telethon.sessions import MemorySession

    session = MemorySession()
    session.set_dc(2, "127.0.0.1", 443)
    session.auth_key = AuthKey(bytes(range(256)))
    client = TelegramClient(session, 12345, "synthetic-api-hash")
    assert client.session is session
    assert type(client.session.auth_key.key_id) is int
    assert client.session.dc_id == 2
    assert not client.is_connected()


async def test_media_preflight_error_does_not_expose_filename(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    await a.start()
    try:
        sentinel = "synthetic-confidential-filename.bin"
        with pytest.raises(TransientRefused) as failure:
            await a.send_file(7, tmp_path / sentinel)
        assert bool(sentinel not in str(failure.value))
        assert failure.value.__suppress_context__
        assert wire.a.sent == []
    finally:
        await a.stop()


async def test_cancellation_resistant_network_reports_incomplete_cleanup(tmp_path, monkeypatch):
    from telethon_secret_chat import TransientCleanupIncomplete

    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(
        wire.a,
        ProtectedFileStorage(tmp_path / "protected.json"),
        limits=TransientLimits(),
    )
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    release, resisted = asyncio.Event(), asyncio.Event()
    try:
        chat, _ = await establish(a, b, wire)
        original = type(wire.a).__call__

        async def resist(client, request):
            from telethon.tl import functions

            if client is wire.a and isinstance(request, functions.messages.SendEncryptedRequest):
                try:
                    await release.wait()
                except asyncio.CancelledError:
                    resisted.set()
                    await release.wait()
                return await original(client, request)
            return await original(client, request)

        monkeypatch.setattr(type(wire.a), "__call__", resist)
        a.limits = TransientLimits(operation_seconds=0.05, cleanup_seconds=0.05)
        with pytest.raises(TransientCleanupIncomplete):
            await a.send_message(chat.id, "synthetic uncertain send")
        assert resisted.is_set()
        assert a.is_suspended(chat.id)
        assert any(not task.done() for task in a._handler_tasks)
        release.set()
        a.limits = TransientLimits()
        await a.settle()
    finally:
        release.set()
        await a.stop()
        await b.stop()


async def test_suspension_write_failure_still_cancels_owned_handler(tmp_path, monkeypatch):
    wire = Wire()
    authorize(wire.a)
    protected = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, protected)
    b = SecretChatManager(wire.b, MemoryStorage())
    entered, finished = asyncio.Event(), asyncio.Event()

    async def waiting(event):
        entered.set()
        try:
            await asyncio.Event().wait()
        finally:
            finished.set()

    a.on("MessageReceived", waiting)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic active callback")
        await asyncio.wait_for(entered.wait(), 1)
        original = protected._write

        def fault():
            raise OSError("synthetic protected commit failure")

        monkeypatch.setattr(protected, "_write", fault)
        with pytest.raises(TransientRefused):
            await a.close(chat.id)
        await a.settle()
        assert finished.is_set()
        assert a.is_suspended(chat.id)
        assert not a._owned
        monkeypatch.setattr(protected, "_write", original)
    finally:
        await a.stop()
        await b.stop()


async def test_gap_completion_and_duplicate_use_original_frames(tmp_path):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    got = []

    async def collect(event):
        got.append(event.text)

    a.on("MessageReceived", collect)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        wire.b.hold = True
        await b.send_message(chat.id, "first")
        await b.send_message(chat.id, "second")
        original_first = wire.b.held[0]
        original_second = wire.b.held[1]
        await wire.a.deliver(*original_second)
        assert not got
        await wire.a.deliver(*original_first)
        await a.settle()
        assert got == ["first", "second"]
        counter = store.load(chat.id)["chat"]["in_seq_no"]
        await wire.a.deliver(*original_second)
        await a.settle()
        assert got == ["first", "second"]
        assert store.load(chat.id)["chat"]["in_seq_no"] == counter
        assert not a._storage.peek_in(chat.id)
        assert not a.is_suspended(chat.id)
    finally:
        await a.stop()
        await b.stop()


async def test_nested_operation_reserves_bytes_without_another_task(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    await a.start()
    try:
        async with a._work(None, 1024):
            before = a._input_bytes
            async with a._work(None, 2048):
                assert a._input_bytes == before + 2048
                assert len(a._owned) == 1
            assert a._input_bytes == before
            with pytest.raises(TransientRefused):
                async with a._work(None, a.limits.bytes):
                    pytest.fail("oversized nested reservation was admitted")
        assert a._input_bytes == 0
    finally:
        await a.stop()


@pytest.mark.parametrize("state", ["requested", "accepted", "committed"])
async def test_rekey_phase_crash_preserves_keys_and_refuses_replay(tmp_path, state):
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        entity = a._entity(chat.id)
        with a._atomic(entity):
            entity.exchange_id = 42
            entity.rekey_role = state
            entity.pending_key = bytes((index * 3) % 256 for index in range(256))
            entity.previous_key = bytes((index * 5) % 256 for index in range(256))
            entity.gap_requested = True
            entity.resend_due = [0, 2]
        before = store.load(chat.id)["chat"]
        await a.stop()
        restored = TransientSecretChatManager(wire.a, store)
        count = len(wire.a.sent)
        await restored.start()
        try:
            assert restored.is_suspended(chat.id)
            after = store.load(chat.id)["chat"]
            for field in (
                "key",
                "pending_key",
                "previous_key",
                "rekey_role",
                "exchange_id",
                "in_seq_no",
                "out_seq_no",
            ):
                assert bool(before[field] == after[field])
            assert len(wire.a.sent) == count
        finally:
            await restored.stop()
    finally:
        await a.stop()
        await b.stop()


async def test_task_budget_rejects_second_waiter_without_unbounded_work(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(
        wire.a, ProtectedFileStorage(tmp_path / "protected.json"), limits=TransientLimits(tasks=1)
    )
    await a.start()
    entered, release = asyncio.Event(), asyncio.Event()

    async def occupy():
        async with a._work(None):
            entered.set()
            await release.wait()

    task = asyncio.create_task(occupy())
    try:
        await asyncio.wait_for(entered.wait(), 1)
        with pytest.raises(TransientRefused):
            await a.create(2000)
        assert len(a._owned) == 1
        assert wire.a.sent == []
    finally:
        release.set()
        await asyncio.wait_for(task, 1)
        await a.stop()


async def test_no_second_delivery_after_an_earlier_handler_suspends(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    deliveries, refusals = [], []

    async def suspend(event):
        await a.close(event.chat_id)

    async def later(event):
        deliveries.append(event.random_id)

    async def diagnostic(event):
        refusals.append(event.chat_id)

    a.on("MessageReceived", suspend)
    a.on("MessageReceived", later)
    a.on("DecryptFailed", diagnostic)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic suspended dispatch")
        await a.settle()
        await a.settle()  # first callback schedules the bounded diagnostic
        assert deliveries == []
        assert refusals == [chat.id]
        assert a.is_suspended(chat.id)
    finally:
        await a.stop()
        await b.stop()


async def test_simultaneous_rekey_collision_uses_existing_protocol(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        wire.a.hold = wire.b.hold = True
        await a.rekey(chat.id)
        await b.rekey(chat.id)
        await wire.a.release()
        await wire.b.release()
        ca, cb = a._entity(chat.id), b._entity(chat.id)
        assert bool(ca.key == cb.key)
        assert ca.exchange_id is cb.exchange_id is None
        assert not a.is_suspended(chat.id)
    finally:
        await a.stop()
        await b.stop()


async def test_no_message_handler_refuses_instead_of_claiming_delivery(tmp_path):
    wire = Wire()
    authorize(wire.a)
    a = TransientSecretChatManager(wire.a, ProtectedFileStorage(tmp_path / "protected.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        await b.send_message(chat.id, "synthetic unhandled message")
        assert a.is_suspended(chat.id)
        assert not wire.a.discarded
    finally:
        await a.stop()
        await b.stop()
