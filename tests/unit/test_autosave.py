"""Auto-save: every message and file of every secret chat, kept on disk (spec 009)."""

import asyncio
import logging

import pytest

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.storage import FileStorage

from .fake_client import Wire, establish


@pytest.fixture
async def saving(pair, tmp_path):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    folder = tmp_path / "saved"
    await a.start_auto_save_secret_chats(folder)
    return wire, a, b, chat_a.id, folder


def kinds(records):
    return [(r["type"], r["out"]) for r in records]


async def test_the_switch_survives_a_restart(tmp_path):
    wire, store = Wire(), tmp_path / "store.json"
    first = SecretChatManager(wire.a, storage=FileStorage(store))
    await first.start()
    assert first.auto_save_secret_chats == str(tmp_path / "saved-secret-chats")
    await first.start_auto_save_secret_chats(tmp_path / "saved")
    await first.stop()

    second = SecretChatManager(wire.a, storage=FileStorage(store))
    await second.start()
    assert second.auto_save_secret_chats == str(tmp_path / "saved")
    await second.stop_auto_save_secret_chats()
    await second.stop()

    third = SecretChatManager(wire.a, storage=FileStorage(store))
    await third.start()
    assert third.auto_save_secret_chats is None
    await third.stop()


async def test_messages_both_ways_self_destructing_and_service_actions(saving):
    _, a, b, chat_id, _ = saving
    await a.send_message(chat_id, "from a")
    await b.set_ttl(chat_id, 5)
    await b.send_message(chat_id, "self-destructing from b")
    records = a.read_saved_messages(chat_id)
    assert kinds(records) == [("message", True), ("service", False), ("message", False)]
    assert records[0]["text"] == "from a"
    assert records[1]["action"]["_"] == "decryptedMessageActionSetMessageTTL"
    assert records[2]["text"] == "self-destructing from b" and records[2]["ttl"] == 5
    assert all(type(r["date"]) is int and r["date"] > 0 for r in records)


async def test_files_both_ways_are_kept_decrypted(saving, tmp_path):
    _, a, b, chat_id, _ = saving
    received, sent = b"a photo from b" * 50, b"a document from a" * 70
    (tmp_path / "in.jpg").write_bytes(received)
    await b.send_file(chat_id, tmp_path / "in.jpg")
    await a.send_file(chat_id, sent, file_name="notes.txt")
    await a._autosave.settle()
    by_direction = {r["out"]: r for r in a.read_saved_messages(chat_id)}
    from pathlib import Path

    assert Path(by_direction[False]["file"]).read_bytes() == received
    assert Path(by_direction[True]["file"]).read_bytes() == sent
    # No file key on disk: the saved file is already plaintext.
    assert all("key" not in r["media"] and "iv" not in r["media"] for r in by_direction.values())
    assert "reference" not in by_direction[False]


async def test_saved_messages_survive_deletes(saving):
    _, a, b, chat_id, _ = saving
    random_id = await b.send_message(chat_id, "keep me")
    await b.delete_messages(chat_id, [random_id])
    await a.delete_secret_chat_both_sides(chat_id)
    assert [r.get("text") for r in a.read_saved_messages(chat_id) if r["type"] == "message"] == [
        "keep me"
    ]


async def test_delete_saved_messages_removes_the_chat_folder(saving):
    _, a, _, chat_id, folder = saving
    await a.send_message(chat_id, "x")
    assert (folder / str(chat_id)).is_dir()
    a.delete_saved_messages(chat_id)
    assert not (folder / str(chat_id)).exists()
    assert a.read_saved_messages(chat_id) == []


async def test_turning_off_stops_saving_and_keeps_what_was_saved(saving):
    _, a, _, chat_id, _ = saving
    await a.send_message(chat_id, "saved")
    await a.stop_auto_save_secret_chats()
    await a.send_message(chat_id, "not saved")
    assert [r["text"] for r in a.read_saved_messages(chat_id)] == ["saved"]


async def test_a_save_failure_never_breaks_delivery(pair, tmp_path, caplog):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    blocked = tmp_path / "a-file"
    blocked.write_bytes(b"")
    await a.start_auto_save_secret_chats(blocked)  # a file, not a folder: every save fails
    got = []
    a.on("MessageReceived", got.append)
    with caplog.at_level(logging.WARNING, logger="telethon_secret_chat"):
        await b.send_message(chat_a.id, "still delivered")
        await a.send_message(chat_a.id, "still sent")
    assert [event.text for event in got] == ["still delivered"]
    assert "auto-save failed" in caplog.text and "still" not in caplog.text


async def test_an_interrupted_download_is_retried_at_the_next_start(saving, tmp_path):
    wire, a, b, chat_id, _ = saving
    release = asyncio.Event()
    original = wire.a.download_file

    async def slow(location, out, **kwargs):
        await release.wait()
        return await original(location, out, **kwargs)

    wire.a.download_file = slow
    await b.send_file(chat_id, b"late bytes" * 20, file_name="late.bin")
    await a.stop()  # cancels the pending download
    wire.a.download_file = original
    await a.start()
    await a._autosave.settle()
    (record,) = [r for r in a.read_saved_messages(chat_id) if r["type"] == "message"]
    from pathlib import Path

    assert Path(record["file"]).read_bytes() == b"late bytes" * 20


@pytest.mark.timeout(10)
async def test_default_file_storage_saves_and_continues_same_chat_after_restart(tmp_path):
    from telethon_secret_chat.storage import MemoryStorage

    wire = Wire()
    path = tmp_path / "state" / "chats.json"
    first = SecretChatManager(wire.a, FileStorage(path))
    peer = SecretChatManager(wire.b, MemoryStorage())
    await first.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(first, peer, wire)
        folder = path.parent / "saved-secret-chats"
        assert first.auto_save_secret_chats == str(folder)
        await first.send_message(chat.id, "before restart")
        before = first.status(chat.id)
        sent_counter = first._storage.load(chat.id)["out_seq_no"]
        await first.stop()
        resumed = SecretChatManager(wire.a, FileStorage(path))
        incoming = []
        resumed.on("MessageReceived", incoming.append)
        await resumed.start()
        assert resumed.status(chat.id).key_fingerprint == before.key_fingerprint
        await resumed.send_message(chat.id, "after restart")
        await peer.send_message(chat.id, "peer after restart")
        assert [message.text for message in incoming] == ["peer after restart"]
        assert resumed._storage.load(chat.id)["out_seq_no"] > sent_counter
        records = resumed.read_saved_messages(chat.id)
        assert [record["text"] for record in records] == [
            "before restart",
            "after restart",
            "peer after restart",
        ]
        assert not wire.a.discarded
    finally:
        if resumed is not None:
            await resumed.stop()
        await first.stop()
        await peer.stop()


@pytest.mark.timeout(10)
async def test_default_auto_save_downloads_files_without_manual_enable(tmp_path):
    from telethon_secret_chat.storage import MemoryStorage

    wire = Wire()
    a = SecretChatManager(wire.a, FileStorage(tmp_path / "state.json"))
    b = SecretChatManager(wire.b, MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        content = b"synthetic auto-saved document"
        await b.send_file(chat.id, content, file_name="sample.bin")
        await a.stop_auto_save_secret_chats()  # waits for the real download
        record = a.read_saved_messages(chat.id)[0]
        from pathlib import Path

        assert Path(record["file"]).read_bytes() == content
        assert not (tmp_path / "saved-secret-chats" / str(chat.id) / "pending.json").exists()
    finally:
        await a.stop()
        await b.stop()


@pytest.mark.timeout(10)
async def test_explicit_off_before_first_start_remains_off_after_restart(tmp_path):
    from telethon_secret_chat.storage import MemoryStorage

    wire = Wire()
    path = tmp_path / "state.json"
    first = SecretChatManager(wire.a, FileStorage(path))
    await first.stop_auto_save_secret_chats()
    await first.start()
    await first.stop()
    second = SecretChatManager(wire.a, FileStorage(path))
    peer = SecretChatManager(wire.b, MemoryStorage())
    await second.start()
    await peer.start()
    try:
        chat, _ = await establish(second, peer, wire)
        await second.send_message(chat.id, "not archived")
        assert second.auto_save_secret_chats is None
        assert not (tmp_path / "saved-secret-chats").exists()
    finally:
        await second.stop()
        await peer.stop()


@pytest.mark.timeout(10)
async def test_relative_store_and_default_archive_stay_anchored_when_cwd_changes(
    tmp_path, monkeypatch
):
    original = tmp_path / "original"
    elsewhere = tmp_path / "elsewhere"
    original.mkdir()
    elsewhere.mkdir()
    monkeypatch.chdir(original)
    wire = Wire()
    storage = FileStorage("state.json")
    manager = SecretChatManager(wire.a, storage)
    monkeypatch.chdir(elsewhere)
    await manager.start()
    try:
        assert storage.path == original / "state.json"
        assert manager.auto_save_secret_chats == str(original / "saved-secret-chats")
        assert not (elsewhere / "state.json").exists()
        assert storage.load_setting("auto_save_secret_chats")["on"] is True
    finally:
        await manager.stop()


@pytest.mark.timeout(10)
async def test_explicit_folder_for_memory_backend_preserves_disabled_preference(tmp_path):
    from telethon_secret_chat.storage import MemoryStorage

    wire, storage = Wire(), MemoryStorage()
    first = SecretChatManager(wire.a, storage, auto_save_folder=tmp_path / "chosen")
    await first.start()
    assert first.auto_save_secret_chats == str(tmp_path / "chosen")
    await first.stop_auto_save_secret_chats()
    await first.stop()
    second = SecretChatManager(wire.a, storage, auto_save_folder=tmp_path / "different")
    await second.start()
    try:
        assert second.auto_save_secret_chats is None
        assert second._autosave.saved_in == tmp_path / "chosen"
    finally:
        await second.stop()


@pytest.mark.timeout(10)
async def test_initial_auto_save_setting_failure_installs_no_update_handler(tmp_path, monkeypatch):
    wire = Wire()
    storage = FileStorage(tmp_path / "state.json")
    manager = SecretChatManager(wire.a, storage)
    original = storage._write

    def fail():
        raise OSError("synthetic setting commit fault")

    monkeypatch.setattr(storage, "_write", fail)
    with pytest.raises(OSError):
        await manager.start()
    assert not wire.a.handlers
    assert not manager._running
    assert storage.load_setting("auto_save_secret_chats") is None
    monkeypatch.setattr(storage, "_write", original)
    await manager.start()
    try:
        assert manager.auto_save_secret_chats == str(tmp_path / "saved-secret-chats")
    finally:
        await manager.stop()
