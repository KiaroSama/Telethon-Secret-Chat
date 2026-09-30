"""Reproductions for the September 30 audit; no network or account credentials."""

import asyncio
import io
import os
from concurrent.futures import ThreadPoolExecutor
from threading import Event

import pytest
from telethon.tl import functions, types

from telethon_secret_chat import files
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import ManagerStopping, StoreCorrupt
from telethon_secret_chat.storage import FileStorage

from .fake_client import FakeClient
from .helpers import incoming, peer_message, ready_manager


def test_short_ciphertext_read_does_not_replace_the_destination(tmp_path):
    key, iv = files.new_file_key()
    cipher = files.encrypt_file(b"x" * 65, key, iv)

    class TruncatedAfterStat(io.BytesIO):
        def read(self, count=-1):
            return super().read(min(count, 16)) if self.tell() < 16 else b""

    target = tmp_path / "document.bin"
    target.write_bytes(b"keep the original")
    with pytest.raises(ValueError, match="length|size|ended"):
        files.save(
            target,
            ciphertext=TruncatedAfterStat(cipher),
            key=key,
            iv=iv,
            size=65,
            claimed_fingerprint=files.file_fingerprint(key, iv),
            chat_id=7,
        )
    assert target.read_bytes() == b"keep the original"


def test_one_save_cannot_unlink_another_active_save(tmp_path, monkeypatch):
    key, iv = files.new_file_key()
    cipher = files.encrypt_file(b"x" * 65, key, iv)
    entered, release = Event(), Event()
    real_decrypt = files.decrypt_stream

    def slow_decrypt(*args):
        if not entered.is_set():
            entered.set()
            assert release.wait(5), "the competing save did not reach its cleanup"
        yield from real_decrypt(*args)

    monkeypatch.setattr(files, "decrypt_stream", slow_decrypt)

    def save(name):
        return files.save(
            tmp_path / name,
            ciphertext=cipher,
            key=key,
            iv=iv,
            size=65,
            claimed_fingerprint=files.file_fingerprint(key, iv),
            chat_id=7,
        )

    with ThreadPoolExecutor(max_workers=2) as executor:
        first = executor.submit(save, "first.bin")
        assert entered.wait(5)
        second = executor.submit(save, "second.bin")
        try:
            second.result(timeout=5)
        finally:
            release.set()
        assert first.result(timeout=5).read_bytes() == b"x" * 65
    assert (tmp_path / "second.bin").read_bytes() == b"x" * 65


def test_opening_another_store_cannot_remove_a_live_commit(tmp_path, monkeypatch):
    store = FileStorage(tmp_path / "one.json")
    real_fsync = os.fsync
    opened = False

    def open_competitor(fd):
        nonlocal opened
        if not opened:
            opened = True
            FileStorage(tmp_path / "two.json")
        return real_fsync(fd)

    monkeypatch.setattr(os, "fsync", open_competitor)
    record = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True).to_record()
    store.save(record)
    assert FileStorage(tmp_path / "one.json").load(7) == record


async def test_early_discard_during_create_is_terminal():
    class RejectingClient(FakeClient):
        async def __call__(self, request):
            result = await super().__call__(request)
            if isinstance(request, functions.messages.RequestEncryptionRequest):
                await manager._on_encryption(types.EncryptedChatDiscarded(id=result.id))
            return result

    manager, _ = ready_manager(RejectingClient())
    chat = await manager.create(2000)
    assert chat.state is ChatState.CLOSED
    assert manager._storage.load(chat.id)["handshake"] == {}


async def test_upload_from_a_previous_run_cannot_overwrite_restarted_state():
    began, resume = asyncio.Event(), asyncio.Event()

    class SlowUpload(FakeClient):
        async def upload_file(self, source, **kwargs):
            began.set()
            await resume.wait()
            return await super().upload_file(source, **kwargs)

    manager, original = ready_manager(SlowUpload())
    await manager.start()
    task = asyncio.create_task(manager.send_file(7, b"payload", file_name="x.bin"))
    await asyncio.wait_for(began.wait(), 2)
    await manager.stop()
    await manager.start()
    current = manager._entity(7)
    await manager._on_encrypted_message(incoming(current, peer_message(0)))
    assert current.in_seq_no == 1
    resume.set()
    try:
        with pytest.raises(ManagerStopping):
            await asyncio.wait_for(task, 2)
        assert manager._storage.load(7)["in_seq_no"] == 1
        assert manager._entity(7).out_seq_no == 0
        assert manager._client.sent_files == []
    finally:
        await manager.stop()


@pytest.mark.parametrize(
    "name,value",
    [
        ("is_outbound", "false"),
        ("state", []),
        ("access_hash", None),
        ("peer_user_id", "9"),
        ("ttl", -1),
        ("ttl", True),
        ("layer", "144"),
        ("wrapper_layer", -1),
        ("created_at", float("nan")),
        ("rekeyed_at", "yesterday"),
        ("handshake", []),
        ("pending_deliveries", "queue"),
        ("gap_requested", "false"),
        ("initial_key_hash", b"short"),
    ],
)
def test_corrupt_runtime_fields_fail_at_the_restore_boundary(name, value):
    _, chat = ready_manager()
    record = chat.to_record()
    record[name] = value
    with pytest.raises(StoreCorrupt):
        SecretChat.from_record(record, stored_id=7)


async def test_new_run_does_not_wait_for_an_old_upload():
    began, resume = asyncio.Event(), asyncio.Event()

    class SlowUpload(FakeClient):
        async def upload_file(self, source, **kwargs):
            began.set()
            await resume.wait()
            return await super().upload_file(source, **kwargs)

    manager, _ = ready_manager(SlowUpload())
    await manager.start()
    old = asyncio.create_task(manager.send_file(7, b"old", file_name="old.bin"))
    await asyncio.wait_for(began.wait(), 2)
    try:
        await manager.stop()
        await manager.start()
        await asyncio.wait_for(manager.send_message(7, "new run"), 1)
        assert manager._storage.load(7)["out_seq_no"] == 1
    finally:
        resume.set()
        with pytest.raises(ManagerStopping):
            await asyncio.wait_for(old, 2)
        await manager.stop()


async def test_concurrent_handler_stops_share_one_shutdown():
    from telethon_secret_chat.events import ChatReady

    manager, _ = ready_manager()
    await manager.start()
    completed = []

    async def stop_handler(event):
        await manager.stop()
        completed.append(True)

    manager.on("ChatReady", stop_handler)
    manager.on("ChatReady", stop_handler)
    manager._emit(ChatReady(7, 9, 0))
    handlers = list(manager._handler_tasks)
    await asyncio.wait_for(asyncio.gather(*handlers), 2)
    assert completed == [True, True]
    assert not manager._running
    await manager.start()
    await manager.send_message(7, "after two concurrent stops")
    await manager.stop()


async def test_cancelling_one_stop_caller_does_not_cancel_shutdown():
    manager, _ = ready_manager()
    await manager.start()
    entered, release = asyncio.Event(), asyncio.Event()

    async def hold_chat():
        async with manager._chat_lock(7):
            entered.set()
            await release.wait()

    blocker = asyncio.create_task(hold_chat())
    await asyncio.wait_for(entered.wait(), 2)
    first = asyncio.create_task(manager.stop())
    await asyncio.sleep(0)
    second = asyncio.create_task(manager.stop())
    await asyncio.sleep(0)
    first.cancel()
    try:
        with pytest.raises(asyncio.CancelledError):
            await first
        assert not second.done()
    finally:
        release.set()
        await blocker
    await asyncio.wait_for(second, 2)
    assert not manager._running


async def test_cancelled_shutdown_waiter_does_not_lose_a_redacted_failure(monkeypatch, caplog):
    manager, _ = ready_manager()
    await manager.start()
    entered, release = asyncio.Event(), asyncio.Event()

    async def hold_chat():
        async with manager._chat_lock(7):
            entered.set()
            await release.wait()

    def fail(chat):
        raise OSError("private storage details")

    blocker = asyncio.create_task(hold_chat())
    await asyncio.wait_for(entered.wait(), 2)
    monkeypatch.setattr(manager, "_save", fail)
    caller = asyncio.create_task(manager.stop())
    await asyncio.sleep(0)
    caller.cancel()
    with pytest.raises(asyncio.CancelledError):
        await caller
    release.set()
    await blocker
    with pytest.raises(OSError):
        await asyncio.wait_for(asyncio.shield(manager._stop_task), 2)
    await asyncio.sleep(0)
    assert "shutdown failed: OSError" in caplog.text
    assert "private storage details" not in caplog.text
