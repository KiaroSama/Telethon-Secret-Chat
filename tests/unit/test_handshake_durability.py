"""The initial exchange across restarts, retries and late updates."""

import pytest
from telethon.tl import functions, types

from telethon_secret_chat import SecretChatManager, crypto, handshake
from telethon_secret_chat.chat import ChatState

from telethon_secret_chat.storage import MemoryStorage

from .dh_material import SAFE_PRIME
from .fake_client import FakeClient
from .helpers import ready_manager


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
        assert resumed._entity(chat.id).key == key
        assert resumed._entity(chat.id).handshake == {}
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
        await resumed.accept(7)
        key = resumed._entity(7).key
        await resumed._on_encryption(requested)
        await resumed.accept(7)
        assert resumed._entity(7).key == key
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
    assert manager._entity(7).key is None
    assert manager._entity(7).state is ChatState.PENDING
    client.failing = False
    await manager.accept(7)
    requests = [
        r for r in client.sent if isinstance(r, functions.messages.AcceptEncryptionRequest)
    ]
    assert requests[-1].g_b == original.g_b
    assert requests[-1].key_fingerprint == original.key_fingerprint


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
    chat = client.manager._entity((await client.manager.create(999)).id)
    assert chat.state is ChatState.READY
    assert chat.key is not None
    assert not client.manager._early_encryption


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


class _AcceptAnswers(FakeClient):
    """The server's answer to acceptEncryption is chosen by the test."""

    error = None

    async def __call__(self, request):
        if self.error is not None and isinstance(
            request, functions.messages.AcceptEncryptionRequest
        ):
            self.sent.append(request)
            raise self.error
        return await super().__call__(request)


async def _pending(client):
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
    return manager


async def test_accept_the_server_already_holds_becomes_ready():
    """A crash between the accept RPC and its commit: the server and the peer hold our
    g_b, the store still says pending with the same secret. Accepting again must
    converge, not fail on ENCRYPTION_ALREADY_ACCEPTED for ever."""
    from telethon import errors as telethon_errors

    client = _AcceptAnswers()
    manager = await _pending(client)
    client.error = telethon_errors.EncryptionAlreadyAcceptedError(request=None)
    ready = []
    manager.on("ChatReady", ready.append)
    await manager.accept(7)
    assert manager._entity(7).state is ChatState.READY
    assert ready


async def test_a_declined_request_closes_the_local_record():
    from telethon import errors as telethon_errors

    from telethon_secret_chat.errors import ChatClosed

    client = _AcceptAnswers()
    manager = await _pending(client)
    client.error = telethon_errors.EncryptionDeclinedError(request=None)
    with pytest.raises(ChatClosed):
        await manager.accept(7)
    assert manager._entity(7).state is ChatState.CLOSED
    assert not any(isinstance(r, functions.messages.DiscardEncryptionRequest) for r in client.sent)
