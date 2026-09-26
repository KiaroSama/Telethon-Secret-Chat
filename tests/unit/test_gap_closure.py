"""Regressions for the audit gaps PR #15 left open (specs/002-audit-gap-closure).

All material is synthetic. Each test names the requirement it pins.
"""

import itertools
import re

import pytest
from telethon.tl import functions

from telethon_secret_chat import SecretChatManager, StoreCorrupt, crypto, framing, rekey, sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .dh_material import SAFE_PRIME
from .fake_client import FakeClient
from .test_replay_and_gap import a_chat, peer_message

KEY = bytes(range(256))
OTHER = bytes(reversed(range(256)))


def a_record(**over):
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    chat.adopt_key(KEY)
    record = chat.to_record()
    record.update(over)
    return record


def store_with(*records, filed_as=None):
    store = MemoryStorage()
    for record in records:
        store.save(record)
    if filed_as is not None:
        record = records[-1]
        del store._state["chats"][str(record["id"])]
        store._state["chats"][str(filed_as)] = record
    return store


# --- FR-001: a corrupt store never half-starts the manager ----------------------


CORRUPTIONS = {
    "short key": dict(key=KEY[:255]),
    "fingerprint disagrees with key": dict(key_fingerprint=crypto.key_fingerprint(OTHER)),
    "unknown state": dict(state="half-open"),
    "ready without a key": dict(key=None, key_fingerprint=None),
    "negative counter": dict(out_seq_no=-1),
    "bool counter": dict(in_seq_no=True),
    "short pending key": dict(pending_key=KEY[:10]),
    "long previous key": dict(previous_key=KEY + b"\x00"),
}


@pytest.mark.parametrize("corruption", CORRUPTIONS, ids=list(CORRUPTIONS))
async def test_a_corrupt_record_refuses_start_and_installs_nothing(corruption):
    client = FakeClient()
    manager = SecretChatManager(client, storage=store_with(a_record(**CORRUPTIONS[corruption])))

    with pytest.raises(StoreCorrupt) as caught:
        await manager.start()

    assert caught.value.chat_id == 7
    assert manager.list() == []
    assert client.handlers == []
    rendered = str(caught.value) + repr(caught.value)
    assert KEY.hex()[:32] not in rendered
    assert re.search(r"[0-9a-fA-F]{32,}", rendered) is None


async def test_a_record_filed_under_another_id_refuses_start():
    manager = SecretChatManager(FakeClient(), storage=store_with(a_record(), filed_as=8))
    with pytest.raises(StoreCorrupt):
        await manager.start()
    assert manager.list() == []


async def test_one_corrupt_record_among_valid_ones_installs_none_of_them():
    good = a_record(id=11)
    bad = a_record(id=12, key=KEY[:100])
    manager = SecretChatManager(FakeClient(), storage=store_with(good, bad))
    with pytest.raises(StoreCorrupt):
        await manager.start()
    assert manager.list() == []


async def test_valid_and_legacy_records_still_start():
    legacy = a_record(id=21)
    for later_field in ("gap_end", "new_key_confirmed", "initial_key_hash", "handshake"):
        legacy.pop(later_field)
    closed = a_record(id=22)
    closed.update(state="closed", key=None, key_fingerprint=None, closed_reason="done")
    manager = SecretChatManager(FakeClient(), storage=store_with(a_record(), legacy, closed))
    await manager.start()
    assert sorted(chat.id for chat in manager.list()) == [7, 21, 22]
    assert manager.status(7).state is ChatState.READY
    await manager.stop()


# --- FR-004: frames refuse a key that is not 256 bytes --------------------------


class _NoCipher:
    @staticmethod
    def encrypt_ige(*_):
        raise AssertionError("cipher reached with an invalid key")

    decrypt_ige = encrypt_ige


@pytest.mark.parametrize("length", [255, 257])
def test_frames_refuse_a_wrong_length_key_before_any_cipher_work(monkeypatch, length):
    frame = crypto.encrypt_frame(KEY, b"body", 0)
    monkeypatch.setattr(crypto, "AES", _NoCipher)
    with pytest.raises(ValueError):
        crypto.encrypt_frame(bytes(length), b"body", 0)
    with pytest.raises(ValueError):
        crypto.decrypt_frame(bytes(length), frame, 0)


# --- FR-005: a failed local save discards the server-side chat ------------------


class _SaveFails(MemoryStorage):
    def _commit(self, chat_id, record):
        raise OSError("disk full")


class _DiscardFails(FakeClient):
    async def __call__(self, request):
        if isinstance(request, functions.messages.DiscardEncryptionRequest):
            self.sent.append(request)
            raise ConnectionError("offline")
        return await super().__call__(request)


@pytest.mark.parametrize("client_type", [FakeClient, _DiscardFails])
async def test_create_discards_the_server_chat_when_the_local_save_fails(client_type):
    client = client_type()
    manager = SecretChatManager(client, storage=_SaveFails())

    with pytest.raises(OSError, match="disk full"):
        await manager.create(2000)

    requested = next(
        r for r in client.sent if isinstance(r, functions.messages.RequestEncryptionRequest)
    )
    discards = [
        r for r in client.sent if isinstance(r, functions.messages.DiscardEncryptionRequest)
    ]
    assert requested is not None
    assert [d.chat_id for d in discards] == [client._next_chat_id]
    assert manager.list() == []


# --- FR-006: service-only traffic still reaches the rekey trigger --------------


def _service_actions(client, chat):
    actions = []
    for request in client.sent:
        if isinstance(request, functions.messages.SendEncryptedServiceRequest):
            body = crypto.decrypt_frame(KEY, request.data, chat.out_x)
            actions.append(type(framing.unwrap(body, chat_id=chat.id).message.action))
    return actions


async def test_a_due_key_is_rekeyed_before_a_service_action():
    client = FakeClient()
    manager = SecretChatManager(client, storage=MemoryStorage())
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    chat.adopt_key(KEY)
    chat.layer = 144
    chat.dh_g, chat.dh_prime = 2, SAFE_PRIME
    chat.messages_since_rekey = rekey.MESSAGE_TRIGGER + 1
    manager._chats[chat.id] = chat
    manager._save(chat)

    await manager.mark_read(chat.id, [1])

    assert _service_actions(client, chat) == [
        tl.DecryptedMessageActionRequestKey,
        tl.DecryptedMessageActionReadMessages,
    ]
    assert chat.state is ChatState.REKEYING


# --- FR-007: every arrival order of four consecutive messages -------------------


@pytest.mark.parametrize("order", list(itertools.permutations(range(4))))
def test_every_arrival_order_of_four_messages_delivers_all_in_sender_order(order):
    chat, store = a_chat(), MemoryStorage()
    delivered = []
    for raw_out in order:
        delivered += [
            w.message.random_id for w in sequence.accept(chat, peer_message(raw_out), store).ready
        ]
    assert delivered == [0, 1, 2, 3]
    assert chat.in_seq_no == 4
    assert chat.state is ChatState.READY
    assert store.peek_in(chat.id) == []
