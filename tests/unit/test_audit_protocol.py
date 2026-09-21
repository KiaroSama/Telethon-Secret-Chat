"""Regressions for the September 2026 audit; all material is synthetic."""

import logging
from copy import deepcopy

import pytest
from telethon.tl import functions, types

from telethon_secret_chat import SecretChatManager, crypto, files, framing, sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.events import ChatReady, MessageReceived
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import FakeClient, Wire, establish
from .test_replay_and_gap import a_chat, peer_message

KEY = bytes(range(256))
OTHER = bytes(reversed(range(256)))


def ready_manager(client=None, store=None):
    manager = SecretChatManager(client or FakeClient(), storage=store or MemoryStorage())
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    chat.adopt_key(KEY)
    chat.layer = 144
    manager._chats[chat.id] = chat
    manager._save(chat)
    return manager, chat


@pytest.fixture
async def pair():
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    yield wire, a, b
    await a.stop()
    await b.stop()


def test_old_replay_does_not_fail_monotonic_echo():
    chat, store = a_chat(sent=5), MemoryStorage()
    sequence.accept(chat, peer_message(0, raw_in=0), store)
    sequence.accept(chat, peer_message(1, raw_in=3), store)
    assert sequence.accept(chat, peer_message(0, raw_in=0), store).ready == []
    assert chat.state is ChatState.READY


def test_gap_echo_is_committed_in_sequence_order():
    chat, store = a_chat(sent=5), MemoryStorage()
    sequence.accept(chat, peer_message(2, raw_in=3), store)
    assert sequence.accept(chat, peer_message(0, raw_in=1), store).ready
    result = sequence.accept(chat, peer_message(1, raw_in=2), store)
    assert len(result.ready) == 2
    assert chat.peer_in_seq_no == 3


def test_drained_gap_is_revalidated_against_predecessor():
    chat, store = a_chat(sent=5), MemoryStorage()
    sequence.accept(chat, peer_message(2, raw_in=1), store)
    sequence.accept(chat, peer_message(0, raw_in=2), store)
    with pytest.raises(MessageRejected):
        sequence.accept(chat, peer_message(1, raw_in=2), store)


def test_second_hole_gets_a_new_resend_span():
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(1), store)
    sequence.accept(chat, peer_message(3), store)
    result = sequence.accept(chat, peer_message(0), store)
    assert result.resend == (4, 4)


@pytest.mark.parametrize("raw_out", [-1, -2])
def test_negative_outbound_counter_is_rejected(raw_out):
    with pytest.raises(MessageRejected):
        sequence.accept(a_chat(), peer_message(raw_out), MemoryStorage())


@pytest.mark.parametrize("end", [2, 4])
def test_resend_end_parity_must_match_start(end):
    chat, store = a_chat(), MemoryStorage()
    store.queue_out(chat.id, {"seq_no": 1, "body": "00"})
    store.queue_out(chat.id, {"seq_no": 3, "body": "00"})
    with pytest.raises(Exception):
        sequence.answer_resend(chat, store, 1, end)


async def test_forged_pending_fingerprint_never_switches_key():
    manager, chat = ready_manager()
    chat.pending_key = OTHER
    chat.exchange_id = 123
    chat.exchange_secret = 456
    chat.rekey_role = "accepted"
    chat.state = ChatState.REKEYING
    before = deepcopy(chat.to_record())
    forged = crypto.key_fingerprint(OTHER).to_bytes(8, "little", signed=True) + b"\x00" * 64
    await manager._on_encrypted_message(
        types.EncryptedMessage(
            random_id=1, chat_id=chat.id, date=0, bytes=forged, file=types.EncryptedFileEmpty()
        )
    )
    assert chat.to_record() == before


async def test_send_commit_failure_rolls_back_chat_and_retention(monkeypatch):
    manager, chat = ready_manager()
    before = deepcopy(chat.to_record())

    def refuse():
        raise OSError("synthetic storage failure")

    monkeypatch.setattr(manager._storage, "_write", refuse)
    with pytest.raises(OSError):
        await manager.send_message(chat.id, "must never get a sequence hole")
    assert chat.to_record() == before
    assert manager._storage.retained_out(chat.id) == []
    assert not manager._client.sent


async def test_outer_random_id_matches_message_random_id():
    manager, chat = ready_manager()
    sent_id = await manager.send_message(chat.id, "identity")
    assert manager._client.sent[-1].random_id == sent_id


async def test_service_messages_use_service_rpc():
    manager, chat = ready_manager()
    await manager.set_ttl(chat.id, 60)
    assert isinstance(manager._client.sent[-1], functions.messages.SendEncryptedServiceRequest)


async def test_rpc_retry_reuses_ciphertext_sequence_and_random_id():
    manager, chat = ready_manager()
    await manager.send_message(chat.id, "retained")
    original = manager._client.sent[-1]
    await manager._resend_retained(chat, manager._storage.retained_out(chat.id)[-1])
    replay = manager._client.sent[-1]
    assert replay.data == original.data
    assert replay.random_id == original.random_id
    assert chat.out_seq_no == 1


async def test_close_scrubs_keys_queues_and_history():
    manager, chat = ready_manager()
    chat.pending_key, chat.previous_key = OTHER, KEY
    chat.exchange_secret = 123456
    await manager.send_message(chat.id, "retained plaintext")
    manager._history[chat.id] = [MessageReceived(chat.id, 1, 1, "history plaintext")]
    await manager.close(chat.id)
    record = manager._storage.load(chat.id)
    assert record["key"] is None
    assert record["pending_key"] is None
    assert record["previous_key"] is None
    assert record["exchange_secret"] is None
    assert manager._storage.retained_out(chat.id) == []
    assert manager.read_history(chat.id) == []


async def test_closed_event_documented_alias_works():
    manager, chat = ready_manager()
    seen = []
    manager.on("ChatClosed", seen.append)
    await manager.close(chat.id)
    assert len(seen) == 1


async def test_handler_exception_does_not_log_secret(caplog):
    manager, _ = ready_manager()
    sentinel = "SYNTHETIC-SECRET-NOT-FOR-LOGGING"

    def bad(event):
        raise RuntimeError(sentinel)

    manager.on("ChatReady", bad)
    with caplog.at_level(logging.ERROR):
        manager._emit(ChatReady(7, 8, 9))
    assert sentinel not in caplog.text
    assert all(record.exc_info is None for record in caplog.records)


async def test_callable_repr_does_not_log_secret(caplog):
    manager, _ = ready_manager()

    class Handler:
        def __call__(self, event):
            raise RuntimeError("failure")

        def __repr__(self):
            return "SYNTHETIC-CREDENTIAL-IN-REPR"

    manager.on("ChatReady", Handler())
    with caplog.at_level(logging.ERROR):
        manager._emit(ChatReady(7, 8, 9))
    assert "SYNTHETIC-CREDENTIAL-IN-REPR" not in caplog.text


async def test_history_limit_zero_is_empty():
    manager, chat = ready_manager()
    manager._history[chat.id] = [MessageReceived(chat.id, 1, 1, "stored")]
    assert manager.read_history(chat.id, 0) == []


async def test_invalid_ttl_does_not_mutate_chat():
    manager, chat = ready_manager()
    with pytest.raises(ValueError):
        await manager.set_ttl(chat.id, -1)
    assert chat.ttl == 0
    assert not manager._client.sent


async def test_generator_delete_ids_are_not_consumed_twice(pair):
    wire, a, b = pair
    ca, cb = await establish(a, b, wire)
    seen = []
    b.on("ServiceActionReceived", seen.append)
    await a.delete_messages(ca.id, (i for i in (11, 22)))
    actions = [
        e.action for e in seen if isinstance(e.action, tl.DecryptedMessageActionDeleteMessages)
    ]
    assert actions[-1].random_ids == [11, 22]


async def test_rekey_commit_is_encrypted_with_old_key(pair):
    wire, a, b = pair
    ca, cb = await establish(a, b, wire)
    old = ca.key
    start = len(wire.a.sent)
    await a.rekey(ca.id)
    commits = []
    for request in wire.a.sent[start:]:
        if not hasattr(request, "data"):
            continue
        for key in (old, ca.key):
            try:
                wrapper = framing.unwrap(crypto.decrypt_frame(key, request.data, ca.out_x))
            except MessageRejected:
                continue
            if isinstance(
                getattr(wrapper.message, "action", None), tl.DecryptedMessageActionCommitKey
            ):
                commits.append((key, request))
    assert len(commits) == 1
    assert commits[0][0] == old


async def test_gap_preserves_each_files_own_envelope(pair, tmp_path):
    wire, a, b = pair
    ca, cb = await establish(a, b, wire)
    wire.a.hold = wire.b.hold = True
    one, two = tmp_path / "one.bin", tmp_path / "two.bin"
    one.write_bytes(b"FIRST")
    two.write_bytes(b"SECOND")
    await a.send_file(ca.id, one)
    await a.send_file(ca.id, two)
    records = list(wire.a.held)
    seen = []
    b.on("MessageReceived", seen.append)
    await wire.b.deliver(*records[1])
    await wire.b.deliver(*records[0])
    assert len(seen) == 2
    assert seen[0].file.id != seen[1].file.id
    assert (await b.save_file(seen[1], tmp_path / "received.bin")).read_bytes() == b"SECOND"


async def test_file_resend_keeps_media_rpc_and_attachment(pair, tmp_path):
    wire, a, b = pair
    ca, cb = await establish(a, b, wire)
    wire.a.hold = True
    path = tmp_path / "file.bin"
    path.write_bytes(b"retained media")
    await a.send_file(ca.id, path)
    original = wire.a.sent[-1]
    await a._resend_retained(ca, a._storage.retained_out(ca.id)[-1])
    assert isinstance(wire.a.sent[-1], functions.messages.SendEncryptedFileRequest)
    assert wire.a.sent[-1].file.id == original.file.id


@pytest.mark.parametrize("size,blob", [(-1, b"0" * 16), (17, b"0" * 16), (1, b"0" * 17)])
def test_invalid_download_length_does_not_replace_existing_file(tmp_path, size, blob):
    key, iv = files.new_file_key()
    target = tmp_path / "existing.bin"
    target.write_bytes(b"ORIGINAL")
    with pytest.raises((MessageRejected, ValueError)):
        files.save(
            target,
            ciphertext=blob,
            key=key,
            iv=iv,
            size=size,
            claimed_fingerprint=files.file_fingerprint(key, iv),
            chat_id=7,
        )
    assert target.read_bytes() == b"ORIGINAL"


def test_trailing_tl_data_is_rejected():
    body = bytes(peer_message(0)) + b"\x00" * 4
    with pytest.raises(MessageRejected):
        framing.unwrap(body)


def test_non_message_inner_constructor_is_rejected():
    wrapper = peer_message(0)
    wrapper.message = tl.DecryptedMessageActionNoop()
    with pytest.raises(MessageRejected):
        framing.unwrap(bytes(wrapper))
