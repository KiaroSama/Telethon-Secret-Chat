"""Guards and branches that no other test reached. No real accounts."""

import logging
import os

import pytest

from telethon_secret_chat import SecretChatManager, sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import ChatNotReady, ManagerStopping, MessageRejected
from telethon_secret_chat.storage import FileStorage, MemoryStorage

from .fake_client import FakeClient, establish
from .helpers import FlakyClient, a_chat, peer_message, ready_manager

# --- the store and the recovery window ----------------------------------------------


def test_a_gap_at_the_window_edge_is_accepted_and_one_past_it_aborts():
    store = MemoryStorage()
    assert sequence.preflight(a_chat(), peer_message(sequence.MAX_RESEND_COUNT), store)
    with pytest.raises(MessageRejected, match="recovery window") as caught:
        sequence.preflight(a_chat(), peer_message(sequence.MAX_RESEND_COUNT + 1), store)
    assert caught.value.fatal is True


def test_a_full_gap_queue_aborts():
    chat, store = a_chat(), MemoryStorage()
    for number in range(1, sequence.MAX_RESEND_COUNT + 1):
        store.queue_in(chat.id, {"seq_no": number + 1000, "body": "", "file": None})
    with pytest.raises(MessageRejected, match="recovery window"):
        sequence.preflight(chat, peer_message(5), store)


@pytest.mark.skipif(os.name == "nt", reason="symlinks need a privilege on Windows")
def test_a_symlinked_store_is_refused(tmp_path):
    real = tmp_path / "real.json"
    FileStorage(real)
    link = tmp_path / "link.json"
    link.symlink_to(real)
    with pytest.raises(ValueError, match="symbolic link"):
        FileStorage(link)


@pytest.mark.parametrize(
    "content",
    [b"\xff\xfe garbage", b"[]", b'{"chats": {}, "out": {}}'],
    ids=["garbage", "list", "missing-in"],
)
def test_a_document_that_is_not_a_store_is_refused(tmp_path, content):
    path = tmp_path / "store.json"
    path.write_bytes(content)
    with pytest.raises(ValueError, match="not a valid storage document"):
        FileStorage(path)


@pytest.mark.skipif(os.name == "nt", reason="Windows cannot fsync a directory")
def test_a_directory_fsync_failure_is_a_warning_not_a_rollback(tmp_path, monkeypatch, caplog):
    import telethon_secret_chat.storage as storage_module

    store = FileStorage(tmp_path / "store.json")
    real_open = os.open

    def refuse_directories(path, flags, *args):
        if os.path.isdir(path):
            raise OSError("no directory fsync here")
        return real_open(path, flags, *args)

    monkeypatch.setattr(storage_module.os, "open", refuse_directories)
    with caplog.at_level(logging.WARNING, logger="telethon_secret_chat"):
        store.save(SecretChat(id=3, access_hash=1, peer_user_id=2, is_outbound=True).to_record())
    assert "directory durability could not be confirmed" in caplog.text
    assert FileStorage(tmp_path / "store.json").load(3) is not None


# --- manager branches -----------------------------------------------------------------


async def test_a_legacy_request_without_its_secret_is_closed_at_start():
    store = MemoryStorage()
    chat = SecretChat(id=4, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.state = ChatState.REQUESTED
    store.save(chat.to_record())
    manager = SecretChatManager(FakeClient(), storage=store)
    closed = []
    manager.on("ChatClosed", closed.append)
    await manager.start()
    try:
        assert manager.status(4).state is ChatState.CLOSED
        assert [event.reason for event in closed] == [
            "legacy pending handshake has no recoverable secret"
        ]
    finally:
        await manager.stop()


async def test_start_reports_durable_work_it_could_not_send(caplog):
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(Exception):
        await manager.send_message(chat.id, "stuck")
    restarted = SecretChatManager(client, storage=manager._storage)
    with caplog.at_level(logging.WARNING, logger="telethon_secret_chat"):
        await restarted.start()
    try:
        assert "durable work awaiting retry" in caplog.text
    finally:
        await restarted.stop()


async def test_accepting_a_request_without_its_public_value_refuses():
    manager, _ = ready_manager()
    chat = SecretChat(id=11, access_hash=1, peer_user_id=2, is_outbound=False)
    chat.state = ChatState.PENDING
    chat.handshake = {"p": 23, "g": 5}
    manager._chats[chat.id] = chat
    with pytest.raises(ChatNotReady):
        await manager.accept(chat.id)


@pytest.mark.parametrize("limit", [-1, "5", 2.0])
def test_read_history_refuses_a_limit_that_is_not_a_count(limit):
    manager, chat = ready_manager()
    with pytest.raises(ValueError):
        manager.read_history(chat.id, limit)


async def test_a_send_while_stopping_refuses():
    manager, chat = ready_manager()
    manager._stopping = True
    with pytest.raises(ManagerStopping, match="stopping"):
        await manager.send_message(chat.id, "late")


async def test_a_peer_delete_and_flush_reach_the_receivers_history(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    first = await a.send_message(chat_a.id, "one")
    await a.send_message(chat_a.id, "two")
    assert [event.text for event in b.read_history(chat_b.id)] == ["one", "two"]
    await a.delete_messages(chat_a.id, [first])
    assert [event.text for event in b.read_history(chat_b.id)] == ["two"]
    await a.flush_history(chat_a.id)
    assert b.read_history(chat_b.id) == []


async def test_save_file_refuses_a_text_and_bad_metadata(pair, tmp_path):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    await a.send_message(chat_a.id, "just text")
    with pytest.raises(MessageRejected, match="carries no file"):
        await b.save_file(got[-1], tmp_path / "x.bin")

    source = tmp_path / "file.bin"
    source.write_bytes(b"content")
    await a.send_file(chat_a.id, source)
    got[-1].media.size = -1  # the fingerprint covers key and iv, not the size
    with pytest.raises(MessageRejected, match="invalid encrypted-file metadata"):
        await b.save_file(got[-1], tmp_path / "y.bin")
