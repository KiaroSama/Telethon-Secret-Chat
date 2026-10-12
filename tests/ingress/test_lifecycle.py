"""Public pre-connect attachment and lifecycle fencing, without any account I/O."""

import logging
import secrets

import pytest
from telethon import TelegramClient
from telethon.sessions import MemorySession
from telethon.crypto import AuthKey

import telethon_secret_chat as api
from .test_frontier import make_box

pytestmark = pytest.mark.timeout(10)


def caller_client():
    session = MemorySession()
    session.set_dc(2, "127.0.0.1", 443)
    session.auth_key = AuthKey(bytes(range(256)))  # Burned fixture, never a Telegram credential.
    return TelegramClient(session, 1, "0" * 32)


async def test_public_prepare_attaches_once_and_stop_fences_ack(tmp_path):
    client = caller_client()
    binding = dict(
        user_id=123,
        auth_key_id=client.session.auth_key.key_id,
        dc_id=2,
        generation="fixture-generation",
    )
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    try:
        await ingress.prepare(
            client, baseline=make_box(logging.getLogger("fixture")).session_state()
        )
        assert client._durable_ingress is ingress
        assert not client.is_connected()
        with pytest.raises((ValueError, api.IngressRefused)):
            await ingress.prepare(client)
        await ingress.stop()
        from telethon.tl import functions

        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=40)])
        assert (await ingress.session_state({}, []))[0]["qts"] == 40
    finally:
        store.close()


@pytest.mark.parametrize("change", ["auth", "dc", "owner"])
async def test_replaced_identity_or_owner_refuses_ack_before_send(tmp_path, change):
    from telethon.tl import functions

    client = caller_client()
    binding = dict(
        user_id=123,
        auth_key_id=client.session.auth_key.key_id,
        dc_id=2,
        generation="fixture-generation",
    )
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    try:
        await ingress.prepare(
            client, baseline=make_box(logging.getLogger("fixture")).session_state()
        )
        if change == "auth":
            client.session.auth_key = AuthKey(bytes(reversed(range(256))))
        elif change == "dc":
            client.session.set_dc(3, "127.0.0.1", 443)
        else:
            store.close()
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=40)])
    finally:
        store.close()


async def test_stop_joins_inflight_replay_before_releasing_store_owner(tmp_path):
    import asyncio
    from telethon._updates import EntityCache
    from telethon.tl import types, functions
    from types import SimpleNamespace
    from unit.fake_client import FakeClient
    from unit.dh_material import SAFE_PRIME
    from .test_frontier import DATE

    class HeldDh(FakeClient):
        def __init__(self):
            super().__init__()
            self.entered = asyncio.Event()

        async def __call__(self, request):
            if isinstance(request, functions.messages.GetDhConfigRequest):
                self.entered.set()
                await asyncio.Event().wait()
            return await super().__call__(request)

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    client = HeldDh()
    client.session = SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456))
    client._mb_entity_cache = EntityCache(self_id=123)
    client._durable_ingress = ingress
    manager = api.StrongSecretChatManager(
        client,
        ingress,
        limits=api.CustodyLimits(
            records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192
        ),
    )
    task = None
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        await manager.start()
        request = types.EncryptedChatRequested(
            id=7,
            access_hash=1,
            date=DATE,
            admin_id=2,
            participant_id=123,
            g_a=pow(2, 1234567, SAFE_PRIME).to_bytes(256, "big"),
        )
        await ingress.capture_live(
            types.Updates(
                updates=[types.UpdateEncryption(request, DATE)],
                users=[],
                chats=[],
                date=DATE,
                seq=2,
            ),
            box,
        )
        task = asyncio.create_task(manager.drain_ingress())
        await client.entered.wait()
        await manager.stop()
        assert task.done()
        assert ingress._manager_owner is None
        assert store.load(7) is None
        assert ingress.pending_batches()[0].cursor == 0
    finally:
        if task is not None:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        if ingress._manager_owner is manager:
            await manager.stop()
        store.close()


@pytest.mark.parametrize("failure", ["missing_baseline", "live", "wrong_baseline", "missing_seam"])
async def test_invalid_public_attachment_never_opens_or_converts_client(tmp_path, failure):
    client = caller_client()
    binding = dict(
        user_id=123,
        auth_key_id=client.session.auth_key.key_id,
        dc_id=2,
        generation="fixture-generation",
    )
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    baseline = make_box(logging.getLogger("fixture")).session_state()
    try:
        if failure == "live":
            client.is_connected = lambda: True
        elif failure == "missing_seam":
            client.configure_durable_ingress = None
        elif failure == "wrong_baseline":
            await ingress.bind(client)
            await ingress.bootstrap(baseline)
        with pytest.raises(api.IngressRefused):
            await ingress.prepare(
                client, baseline=None if failure == "missing_baseline" else baseline
            )
        assert client._durable_ingress is None
    finally:
        store.close()


async def test_second_manager_cannot_claim_existing_ingress_owner(tmp_path):
    from .test_protocol_replay import client_for

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    client = client_for(ingress)
    budget = api.CustodyLimits(records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192)
    manager = api.StrongSecretChatManager(client, ingress, limits=budget)
    try:
        await ingress.bind(client)
        await ingress.bootstrap(make_box(logging.getLogger("fixture")).session_state())
        await manager.start()
        with pytest.raises(api.IngressRefused):
            api.StrongSecretChatManager(client, ingress, limits=budget)
        assert ingress._manager_owner is manager
    finally:
        await manager.stop()
        store.close()
