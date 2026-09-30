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
    assert first.auto_save_secret_chats is None
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
