"""Atomic protocol application and consumer custody survive reconstruction."""

import logging
import secrets
from types import SimpleNamespace

import pytest
from telethon._updates import EntityCache
from telethon.tl import types

import telethon_secret_chat as api
from telethon_secret_chat import crypto
from unit.fake_client import FakeClient
from unit.helpers import peer_message, a_chat
from .test_frontier import DATE, make_box

pytestmark = pytest.mark.timeout(10)


def client_for(ingress):
    client = FakeClient()
    client.session = SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456))
    client._mb_entity_cache = EntityCache(self_id=123)
    client._durable_ingress = ingress  # Fixture binds a development contract, not activation.
    return client


async def test_receive_commit_cursor_and_relay_restart_keep_original_identity(tmp_path):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "custody.bin"
    ingress_limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    custody_limits = api.CustodyLimits(
        records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192
    )
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    ingress = api.DurableIngress(store, limits=ingress_limits)
    client = client_for(ingress)
    manager = api.StrongSecretChatManager(client, ingress, limits=custody_limits)
    resumed = None
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        await manager.start()
        chat = a_chat()
        chat.layer = 144
        manager._chats[chat.id] = chat  # Burned synthetic protocol fixture, never an account.
        manager._save(chat)
        wrapper = peer_message(0, text="committed synthetic payload")
        wrapper.message.random_id = 37
        frame = crypto.encrypt_frame(chat.key, bytes(wrapper), chat.in_x)
        message = types.EncryptedMessage(
            random_id=37, chat_id=chat.id, date=DATE, bytes=frame, file=types.EncryptedFileEmpty()
        )
        envelope = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )
        await ingress.capture_live(envelope, box)
        await manager.drain_ingress()
        held = manager.pending_deliveries(chat.id)
        assert len(held) == 1
        assert held[0].message.random_id == 37
        assert held[0].message.text == "committed synthetic payload"
        assert store.load(chat.id)["in_seq_no"] == 1
        assert ingress.pending_batches() == []
        await manager.stop()
        store.close()
        store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
        resumed_ingress = api.DurableIngress(store, limits=ingress_limits)
        resumed_client = client_for(resumed_ingress)
        await resumed_ingress.bind(resumed_client)
        resumed = api.StrongSecretChatManager(
            resumed_client, resumed_ingress, limits=custody_limits
        )
        await resumed.start()
        assert resumed.pending_deliveries(chat.id) == held
        assert store.load(chat.id)["in_seq_no"] == 1
        resumed.acknowledge_delivery(chat.id, held[0].seq_no)
        assert resumed.pending_deliveries(chat.id) == []
    finally:
        if resumed is not None:
            await resumed.stop()
        if not manager._stopping:
            await manager.stop()
        store.close()


@pytest.mark.parametrize("ordinary_first", [False, True])
async def test_rekey_response_commit_settles_mailbox_before_uncertain_send(
    tmp_path, ordinary_first
):
    from telethon.tl import functions
    from telethon_secret_chat import sequence
    from telethon_secret_chat.schema import secret_tl as tl
    from unit.dh_material import SAFE_PRIME

    class UncertainService(FakeClient):
        async def __call__(self, request):
            if isinstance(request, functions.messages.SendEncryptedServiceRequest):
                self.sent.append(request)
                raise OSError("synthetic uncertain service send")
            return await super().__call__(request)

    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "custody.bin"
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    budget = api.CustodyLimits(records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192)
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    ingress = api.DurableIngress(store, limits=limits)
    client = UncertainService()
    client.session = SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456))
    client._mb_entity_cache = EntityCache(self_id=123)
    client._durable_ingress = ingress
    manager = api.StrongSecretChatManager(client, ingress, limits=budget)
    try:
        await ingress.bind(client)
        await ingress.bootstrap(make_box(logging.getLogger("fixture")).session_state())
        await manager.start()
        chat = a_chat()
        chat.layer = 144
        chat.dh_prime, chat.dh_g = SAFE_PRIME, 2
        manager._chats[chat.id] = chat
        wrapper = peer_message(0)
        wrapper.message = tl.DecryptedMessageService(
            random_id=37,
            action=tl.DecryptedMessageActionRequestKey(
                exchange_id=5, g_a=pow(2, 1234567, SAFE_PRIME).to_bytes(256, "big")
            ),
        )
        chat.pending_deliveries = [sequence.pack(wrapper)]
        if ordinary_first:
            chat.pending_deliveries.insert(0, sequence.pack(peer_message(0, text="prior relay")))
        manager._save(chat)
        with pytest.raises(api.SendPending):
            await manager._drain_deliveries(chat)
        persisted = store.load(chat.id)
        assert persisted["pending_deliveries"] == []
        assert persisted["rekey_role"] == "accepted"
        held = store.retained_out(chat.id)
        assert len(held) == 1 and held[0]["pending"]
        assert held[0]["random_id"] == client.sent[-1].random_id
        assert bytes.fromhex(held[0]["frame"]) == client.sent[-1].data
        # Draining again cannot create a fresh AcceptKey identity.
        await manager._drain_deliveries(chat)
        assert store.retained_out(chat.id) == held
    finally:
        await manager.stop()
        store.close()


@pytest.mark.parametrize("stage", ["before_protocol", "after_protocol", "after_relay"])
async def test_reconstruct_each_protocol_relay_crash_barrier_without_new_identity(
    tmp_path, monkeypatch, stage
):
    import os
    from contextlib import contextmanager

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    key = secrets.token_bytes(32)
    path = tmp_path / "custody.bin"
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    budget = api.CustodyLimits(records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192)
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    ingress = api.DurableIngress(store, limits=limits)
    client = client_for(ingress)

    class CrashBarrier(api.StrongSecretChatManager):
        @contextmanager
        def _atomic(self, chat):
            with super()._atomic(chat):
                yield
            if stage == "after_protocol" and self._applied_current:
                raise OSError("synthetic crash after protocol commit")
            if stage == "after_relay" and self.pending_deliveries(chat.id):
                raise OSError("synthetic crash after relay projection")

    manager = CrashBarrier(client, ingress, limits=budget)
    resumed = None
    real_replace = os.replace
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        await manager.start()
        chat = a_chat()
        chat.layer = 144
        manager._chats[chat.id] = chat
        manager._save(chat)
        wrapper = peer_message(0, text="boundary fixture")
        wrapper.message.random_id = 77
        message = types.EncryptedMessage(
            random_id=77,
            chat_id=chat.id,
            date=DATE,
            bytes=crypto.encrypt_frame(chat.key, bytes(wrapper), chat.in_x),
            file=types.EncryptedFileEmpty(),
        )
        await ingress.capture_live(
            types.Updates(
                updates=[types.UpdateNewEncryptedMessage(message, 41)],
                users=[],
                chats=[],
                date=DATE,
                seq=2,
            ),
            box,
        )
        if stage == "before_protocol":

            def refuse_replace(*args):
                raise OSError("synthetic crash before protocol commit")

            monkeypatch.setattr(os, "replace", refuse_replace)
        try:
            await manager.drain_ingress()
        except (api.IngressRefused, OSError):
            pass
        monkeypatch.setattr(os, "replace", real_replace)
        try:
            await manager.stop()
        except api.IngressRefused:
            if stage != "before_protocol":
                raise
        store.close()
        store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
        recovered = api.DurableIngress(store, limits=limits)
        recovered_client = client_for(recovered)
        await recovered.bind(recovered_client)
        resumed = api.StrongSecretChatManager(recovered_client, recovered, limits=budget)
        await resumed.start()
        if resumed.pause_status(chat.id).paused:
            assert resumed.pause_status(chat.id).resumable
            await resumed.resume_chat(chat.id)
        held = resumed.pending_deliveries(chat.id)
        assert len(held) == 1 and held[0].message.random_id == 77
        assert held[0].message.text == "boundary fixture"
        assert store.load(chat.id)["in_seq_no"] == 1
        assert recovered.pending_batches() == []
        resumed.acknowledge_delivery(chat.id, held[0].seq_no)
        assert resumed.pending_deliveries(chat.id) == []
    finally:
        monkeypatch.setattr(os, "replace", real_replace)
        if resumed is not None:
            await resumed.stop()
        if not manager._stopping:
            await manager.stop()
        store.close()
