"""Crash, cancellation, parser and transport boundary regressions; no real accounts."""

import asyncio
import hashlib
import struct
import subprocess
import sys
from copy import deepcopy

import pytest
from telethon.extensions import BinaryReader
from telethon.tl import functions, types

from telethon_secret_chat import SecretChatManager, crypto, files, framing, handshake, rekey
from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.events import ChatReady, MessageReceived, ServiceActionReceived
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import FileStorage, MemoryStorage

from .dh_material import SAFE_PRIME
from .fake_client import FakeClient, Wire, establish
from .test_audit_protocol import KEY, OTHER, ready_manager
from .test_replay_and_gap import peer_message

SEND_TYPES = (
    functions.messages.SendEncryptedRequest,
    functions.messages.SendEncryptedServiceRequest,
    functions.messages.SendEncryptedFileRequest,
)


class FlakyClient(FakeClient):
    failing = True

    async def __call__(self, request):
        if self.failing and isinstance(request, SEND_TYPES):
            self.sent.append(request)
            raise OSError("synthetic RPC uncertainty")
        return await super().__call__(request)


def incoming(chat, wrapper, key=KEY):
    return types.EncryptedMessage(
        random_id=999,
        chat_id=chat.id,
        date=0,
        bytes=crypto.encrypt_frame(key, bytes(wrapper), chat.in_x),
        file=types.EncryptedFileEmpty(),
    )


async def test_restart_retries_uncertain_send_with_exact_original_wire_identity():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(OSError):
        await manager.send_message(chat.id, "uncertain send")
    original = client.sent[-1]
    assert manager._storage.retained_out(chat.id)[0]["pending"]
    client.failing = False
    recovered = SecretChatManager(client, storage=manager._storage)
    await recovered.start()
    try:
        retry = client.sent[-1]
        assert (retry.random_id, retry.data) == (original.random_id, original.data)
        assert recovered.status(chat.id).out_seq_no == 1
        assert not recovered._storage.retained_out(chat.id)[0]["pending"]
    finally:
        await recovered.stop()


async def test_delete_uncertain_message_never_retries_its_plaintext_first():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(OSError):
        await manager.send_message(chat.id, "DELETE-ME-SYNTHETIC")
    random_id = client.sent[-1].random_id
    client.failing = False
    await manager.delete_messages(chat.id, [random_id])
    for request in client.sent[1:]:
        wrapper = framing.unwrap(crypto.decrypt_frame(KEY, request.data, chat.out_x))
        assert not getattr(wrapper.message, "message", "")
        assert isinstance(wrapper.message.action, tl.DecryptedMessageActionDeleteMessages)
    assert all(
        "DELETE-ME-SYNTHETIC".encode().hex() not in item["body"]
        for item in manager._storage.retained_out(chat.id)
    )


async def test_flush_uncertain_message_removes_content_even_if_rpc_stays_down():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(OSError):
        await manager.send_message(chat.id, "FLUSH-ME-SYNTHETIC")
    manager._history[chat.id] = [MessageReceived(chat.id, 1, 0, "history")]
    with pytest.raises(OSError):
        await manager.flush_history(chat.id)
    assert manager.read_history(chat.id) == []
    assert all(
        "FLUSH-ME-SYNTHETIC".encode().hex() not in item["body"]
        for item in manager._storage.retained_out(chat.id)
    )


async def test_cancelled_send_remains_retryable():
    entered = asyncio.Event()

    class Blocking(FakeClient):
        block = True

        async def __call__(self, request):
            if self.block and isinstance(request, SEND_TYPES):
                self.sent.append(request)
                entered.set()
                await asyncio.Future()
            return await super().__call__(request)

    client = Blocking()
    manager, chat = ready_manager(client)
    task = asyncio.create_task(manager.send_message(chat.id, "cancelled RPC"))
    await entered.wait()
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    original = client.sent[-1]
    client.block = False
    await manager.retry_pending(chat.id)
    assert client.sent[-1].data == original.data
    assert client.sent[-1].random_id == original.random_id
    assert chat.out_seq_no == 1
    assert not manager._inflight


async def test_concurrent_send_rpcs_do_not_overlap_or_reorder():
    class Yielding(FakeClient):
        active = 0
        maximum = 0

        async def __call__(self, request):
            if not isinstance(request, SEND_TYPES):
                return await super().__call__(request)
            self.active += 1
            self.maximum = max(self.maximum, self.active)
            try:
                await asyncio.sleep(0)
                return await super().__call__(request)
            finally:
                self.active -= 1

    client = Yielding()
    manager, chat = ready_manager(client)
    ids = await asyncio.gather(*(manager.send_message(chat.id, str(n)) for n in range(20)))
    assert client.maximum == 1
    assert len(set(ids)) == 20
    wrappers = [
        framing.unwrap(crypto.decrypt_frame(KEY, item.data, chat.out_x)) for item in client.sent
    ]
    assert [item.out_seq_no for item in wrappers] == list(range(1, 40, 2))
    assert [item.message.message for item in wrappers] == [str(n) for n in range(20)]


async def test_initial_outbound_handshake_survives_restart():
    client, store = FakeClient(), MemoryStorage()
    manager = SecretChatManager(client, storage=store)
    await manager.start()
    chat = await manager.create(2000)
    ga = client.sent[-1].g_a
    await manager.stop()
    resumed = SecretChatManager(client, storage=store)
    await resumed.start()
    try:
        b = handshake.generate_secret()
        key = handshake.shared_key(peer_value=int.from_bytes(ga, "big"), secret=b, p=SAFE_PRIME)
        await resumed._on_encryption(
            types.EncryptedChat(
                id=chat.id,
                access_hash=7777,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a_or_b=pow(2, b, SAFE_PRIME).to_bytes(256, "big"),
                key_fingerprint=crypto.key_fingerprint(key),
            )
        )
        assert resumed.status(chat.id).key == key
        assert resumed.status(chat.id).handshake == {}
    finally:
        await resumed.stop()


async def test_inbound_pending_handshake_survives_restart_and_duplicate_request():
    client, store = FakeClient(), MemoryStorage()
    manager = SecretChatManager(client, storage=store)
    await manager.start()
    a = handshake.generate_secret()
    requested = types.EncryptedChatRequested(
        id=7,
        access_hash=8,
        date=0,
        admin_id=1000,
        participant_id=2000,
        g_a=pow(2, a, SAFE_PRIME).to_bytes(256, "big"),
    )
    await manager._on_encryption(requested)
    await manager.stop()
    resumed = SecretChatManager(client, storage=store)
    await resumed.start()
    try:
        chat = await resumed.accept(7)
        key = chat.key
        await resumed._on_encryption(requested)
        await resumed.accept(7)
        assert chat.key == key
        assert (
            len(
                [
                    r
                    for r in client.sent
                    if isinstance(r, functions.messages.AcceptEncryptionRequest)
                ]
            )
            == 1
        )
    finally:
        await resumed.stop()


async def test_accept_rpc_failure_keeps_same_private_exponent_on_retry():
    class Failure(FakeClient):
        failing = True

        async def __call__(self, request):
            if self.failing and isinstance(request, functions.messages.AcceptEncryptionRequest):
                self.sent.append(request)
                raise OSError("synthetic accept failure")
            return await super().__call__(request)

    client = Failure()
    manager = SecretChatManager(client, storage=MemoryStorage())
    await manager._on_encryption(
        types.EncryptedChatRequested(
            id=7,
            access_hash=8,
            date=0,
            admin_id=1000,
            participant_id=2000,
            g_a=pow(2, handshake.generate_secret(), SAFE_PRIME).to_bytes(256, "big"),
        )
    )
    with pytest.raises(OSError):
        await manager.accept(7)
    original = client.sent[-1]
    assert manager.status(7).key is None
    assert manager.status(7).state is ChatState.PENDING
    client.failing = False
    await manager.accept(7)
    requests = [
        r for r in client.sent if isinstance(r, functions.messages.AcceptEncryptionRequest)
    ]
    assert requests[-1].g_b == original.g_b
    assert requests[-1].key_fingerprint == original.key_fingerprint


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
        assert not recovered.status(chat.id).pending_deliveries
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


async def test_closed_chat_cannot_be_revived_by_late_key_update():
    manager, chat = ready_manager()
    await manager.close(chat.id)
    await manager._on_encryption(
        types.EncryptedChat(
            id=chat.id,
            access_hash=8,
            date=0,
            admin_id=1000,
            participant_id=2000,
            g_a_or_b=b"not processed",
            key_fingerprint=0,
        )
    )
    assert chat.state is ChatState.CLOSED
    assert chat.key is None


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
    await entered.wait()
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


async def test_file_storage_close_removes_live_key_material(tmp_path):
    path = tmp_path / "private.json"
    manager, chat = ready_manager(store=FileStorage(path))
    await manager.send_message(chat.id, "delete stored plaintext")
    await manager.close(chat.id)
    assert KEY.hex() not in path.read_text()
    assert "delete stored plaintext".encode().hex() not in path.read_text()
    assert FileStorage(path).load(chat.id)["key"] is None


@pytest.mark.parametrize(
    "layer,constructor",
    [(73, tl.DecryptedMessageMediaDocument_7afe8ae2), (143, tl.DecryptedMessageMediaDocument)],
)
async def test_media_constructor_matches_negotiated_layer(tmp_path, layer, constructor):
    manager, chat = ready_manager()
    chat.layer = layer
    path = tmp_path / "one.bin"
    path.write_bytes(b"sample")
    await manager.send_file(chat.id, path)
    wrapper = framing.unwrap(crypto.decrypt_frame(KEY, manager._client.sent[-1].data, chat.out_x))
    assert type(wrapper.message.media) is constructor


async def test_big_file_upload_uses_big_encrypted_handle(tmp_path):
    class Big(FakeClient):
        async def upload_file(self, file, **kwargs):
            return types.InputFileBig(id=5, parts=22, name="big.bin")

    manager, chat = ready_manager(Big())
    path = tmp_path / "small-fixture.bin"
    path.write_bytes(b"a small stand-in for the uploader's BIG result")
    await manager.send_file(chat.id, path)
    assert isinstance(manager._client.sent[-1].file, types.InputEncryptedFileBigUploaded)


async def test_receive_uses_file_dc_and_rejects_bad_fingerprint_before_download(tmp_path):
    class Recording(FakeClient):
        downloads = []

        async def download_file(self, location, out, **kwargs):
            self.downloads.append(kwargs)
            return await super().download_file(location, out, **kwargs)

    client = Recording()
    manager, chat = ready_manager(client)
    key, iv = files.new_file_key()
    client.stored[5] = files.encrypt_file(b"sample", key, iv)
    attached = types.EncryptedFile(
        id=5, access_hash=6, size=16, dc_id=4, key_fingerprint=files.file_fingerprint(key, iv)
    )
    media = tl.DecryptedMessageMediaDocument(key=key, iv=iv, size=6)
    event = MessageReceived(chat.id, 1, 1, "", media=media, file=attached)
    assert (await manager.save_file(event, tmp_path / "ok")).read_bytes() == b"sample"
    assert client.downloads == [{"dc_id": 4}]
    attached.key_fingerprint ^= 1
    with pytest.raises(MessageRejected):
        await manager.save_file(event, tmp_path / "bad")
    assert len(client.downloads) == 1


@pytest.mark.parametrize("size", [0, 1, 15, 16, 17, 255, 256, 257])
def test_file_length_boundaries_are_exact(size):
    key, iv = files.new_file_key()
    data = b"x" * size
    assert files.decrypt_file(files.encrypt_file(data, key, iv), key, iv, size) == data


def test_malformed_ciphertext_is_rejected_without_native_process_abort():
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            "from telethon_secret_chat.files import decrypt_file\n"
            "try: decrypt_file(b'x'*17, b'k'*32, b'i'*32, 1)\n"
            "except ValueError: pass\n"
            "else: raise AssertionError('malformed ciphertext accepted')\n",
        ],
        capture_output=True,
        timeout=15,
    )
    assert result.returncode == 0, "invalid input reached the native cipher or was accepted"


@pytest.mark.parametrize("count", [-1, 2**31 - 1])
def test_generated_parser_rejects_impossible_vector_counts(count):
    data = struct.pack(
        "<IIi", tl.DecryptedMessageActionReadMessages.CONSTRUCTOR_ID, 0x1CB5C415, count
    )
    with BinaryReader(data) as reader, pytest.raises((ValueError, BufferError, struct.error)):
        tl.read_object(reader)


def test_generated_parser_does_not_repair_invalid_utf8():
    data = bytes(tl.DecryptedMessage(random_id=1, ttl=0, message="x"))
    data = data[:-4] + b"\x01\xff\x00\x00"
    with BinaryReader(data) as reader, pytest.raises(UnicodeDecodeError):
        tl.read_object(reader)


@pytest.mark.parametrize(
    "mime", ["audio/ogg-invalid", "audio/opus-invalid", "audio/x-opus-invalid"]
)
def test_voice_mime_is_exact_not_a_prefix(mime):
    with pytest.raises(ValueError):
        files.resolve_kind("voice_note", file_name="voice.ogg", mime_type=mime)


def test_visual_hash_is_not_wire_fingerprint_and_survives_rekey():
    manager, chat = ready_manager()
    expected = hashlib.sha1(KEY).digest()[:16] + hashlib.sha256(KEY).digest()[:20]
    assert chat.key_hash == expected
    rekey.adopt_new_key(chat, OTHER)
    assert chat.key_hash == expected
    manager._save(chat)
    assert manager._storage.load(chat.id)["initial_key_hash"] == expected


def test_service_event_repr_never_includes_action_repr():
    class Sensitive:
        def __repr__(self):
            return "SYNTHETIC-DO-NOT-PRINT"

    assert "SYNTHETIC-DO-NOT-PRINT" not in repr(ServiceActionReceived(1, "action", Sensitive()))


async def test_legacy_resend_without_original_wire_metadata_fails_closed():
    from telethon_secret_chat.errors import ResendUnsatisfiable

    manager, chat = ready_manager()
    wrapper = framing.wrap(
        tl.DecryptedMessage(random_id=123, ttl=0, message="legacy"),
        layer=73,
        in_seq_no=0,
        out_seq_no=1,
    )
    item = {"seq_no": 1, "body": bytes(wrapper).hex()}
    manager._storage.queue_out(chat.id, item)
    with pytest.raises(ResendUnsatisfiable):
        await manager._transmit(chat, item)
    assert chat.state is ChatState.CLOSED
    assert not any(isinstance(request, SEND_TYPES) for request in manager._client.sent)
    assert manager._storage.retained_out(chat.id) == []


async def test_initial_key_update_before_request_rpc_returns_is_not_lost():
    class EarlyClient(FakeClient):
        manager = None

        async def __call__(self, request):
            result = await super().__call__(request)
            if isinstance(request, functions.messages.RequestEncryptionRequest):
                secret = handshake.generate_secret()
                key = handshake.shared_key(
                    peer_value=int.from_bytes(request.g_a, "big"),
                    secret=secret,
                    p=SAFE_PRIME,
                    chat_id=result.id,
                )
                await self.manager._on_encryption(
                    types.EncryptedChat(
                        id=result.id,
                        access_hash=result.access_hash,
                        date=0,
                        admin_id=result.admin_id,
                        participant_id=result.participant_id,
                        g_a_or_b=handshake.public_value(2, secret, SAFE_PRIME).to_bytes(
                            256, "big"
                        ),
                        key_fingerprint=crypto.key_fingerprint(key),
                    )
                )
            return result

    client = EarlyClient()
    client.manager = SecretChatManager(client, storage=MemoryStorage())
    chat = await client.manager.create(999)
    assert chat.state is ChatState.READY
    assert chat.key is not None
    assert not client.manager._early_encryption


def test_full_gap_buffer_can_be_drained_by_the_missing_predecessor():
    from telethon_secret_chat import sequence
    from .test_audit_protocol import a_chat

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


async def test_a_pending_request_stored_across_a_restart_is_announced_again():
    """DD-06. The handshake survives a restart (PR #15), but ``ChatRequested`` was
    emitted only when the update first arrived. An application that accepts from
    that event - telegram-mcp does - never heard about the stored request again,
    so the chat sat ``pending`` until the peer gave up."""
    client, store = FakeClient(), MemoryStorage()
    manager = SecretChatManager(client, storage=store)
    await manager.start()
    await manager._on_encryption(
        types.EncryptedChatRequested(
            id=7,
            access_hash=8,
            date=0,
            admin_id=1000,
            participant_id=2000,
            g_a=pow(2, handshake.generate_secret(), SAFE_PRIME).to_bytes(256, "big"),
        )
    )
    await manager.stop()

    resumed = SecretChatManager(client, storage=store)
    seen = []
    resumed.on("ChatRequested", seen.append)
    await resumed.start()
    try:
        assert [(event.chat_id, event.peer_user_id) for event in seen] == [(7, 1000)]
    finally:
        await resumed.stop()
