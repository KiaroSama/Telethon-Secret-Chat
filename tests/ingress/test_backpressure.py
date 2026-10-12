"""A paused peer retains custody while healthy peers use the finite shared spool."""

import logging
import secrets

import pytest
from telethon.tl import types, functions
from telethon_secret_chat import crypto
import telethon_secret_chat as api
from unit.helpers import a_chat, peer_message
from .test_protocol_replay import client_for
from .test_frontier import DATE, make_box

pytestmark = pytest.mark.timeout(10)


@pytest.mark.parametrize("separate_batches", [False, True])
async def test_paused_peer_does_not_block_healthy_committed_peer_and_replays_once(
    tmp_path, separate_batches
):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "custody.bin", key=key, binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=64, bytes=1024 * 1024, age_seconds=3600)
    )
    client = client_for(ingress)
    manager = api.StrongSecretChatManager(
        client,
        ingress,
        limits=api.CustodyLimits(
            records=32, bytes=1024 * 1024, age_seconds=3600, frame_bytes=8192
        ),
    )
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        await manager.start()
        first, second = a_chat(), a_chat()
        second.id = 8
        for chat in (first, second):
            chat.layer = 144
            manager._chats[chat.id] = chat
            manager._save(chat)
        manager.pause_chat(first.id)
        updates = []
        for qts, chat, identity in ((41, first, 101), (42, second, 102)):
            wrapper = peer_message(0, text=f"fixture {identity}")
            wrapper.message.random_id = identity
            message = types.EncryptedMessage(
                random_id=identity,
                chat_id=chat.id,
                date=DATE,
                bytes=crypto.encrypt_frame(chat.key, bytes(wrapper), chat.in_x),
                file=types.EncryptedFileEmpty(),
            )
            updates.append(types.UpdateNewEncryptedMessage(message=message, qts=qts))
        if separate_batches:
            from telethon._updates import EntityCache

            for offset, update in enumerate(updates):
                envelope = types.Updates(
                    updates=[update], users=[], chats=[], date=DATE, seq=2 + offset
                )
                await ingress.capture_live(envelope, box)
                box.process_updates(envelope, EntityCache(), [])
        else:
            await ingress.capture_live(
                types.Updates(updates=updates, users=[], chats=[], date=DATE, seq=2), box
            )
        await manager.drain_ingress()
        assert manager.pending_deliveries(first.id) == []
        assert manager.pending_deliveries(second.id)[0].message.random_id == 102
        assert ingress.pending_batches()[0].cursor == 0
        assert ingress.pending_batches()[0].applied == (() if separate_batches else (1,))
        if separate_batches:
            assert ingress.pending_batches()[1].applied == (0,)
        await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=42)])
        await manager.resume_chat(first.id)
        assert manager.pending_deliveries(first.id)[0].message.random_id == 101
        assert len(manager.pending_deliveries(second.id)) == 1
        assert ingress.pending_batches() == []
    finally:
        await manager.stop()
        store.close()


async def test_full_shared_spool_preserves_custody_and_stops_all_new_ack(tmp_path):
    from types import SimpleNamespace
    from telethon._updates import EntityCache

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=1, bytes=1024 * 1024, age_seconds=3600)
    )
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())

        def envelope(qts, seq):
            message = types.EncryptedMessage(
                random_id=qts,
                chat_id=7,
                date=DATE,
                bytes=b"bounded fixture",
                file=types.EncryptedFileEmpty(),
            )
            return types.Updates(
                updates=[types.UpdateNewEncryptedMessage(message, qts)],
                users=[],
                chats=[],
                date=DATE,
                seq=seq,
            )

        first = envelope(41, 2)
        await ingress.capture_live(first, box)
        before = ingress.pending_batches()
        box.process_updates(first, EntityCache(), [])
        with pytest.raises(api.IngressRefused):
            await ingress.capture_live(envelope(42, 3), box)
        assert ingress.pending_batches() == before
        assert ingress.checkpoint()["qts"] == 41
        for request in (
            functions.messages.ReceivedQueueRequest(max_qts=41),
            functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=41),
        ):
            with pytest.raises(api.IngressRefused):
                await ingress.guard_requests([request])
    finally:
        store.close()


async def test_expired_spool_preserves_bytes_and_refuses_resume_ack(tmp_path, monkeypatch):
    import time
    from types import SimpleNamespace
    from telethon._updates import EntityCache

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "state.bin"
    store = api.EncryptedIngressStorage(
        path, key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=10)
    )
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"age fixture",
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
        before = path.read_bytes()
        now = time.time()
        monkeypatch.setattr(time, "time", lambda: now + 11)
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
        assert path.read_bytes() == before
        assert len(ingress.pending_batches()) == 1
    finally:
        store.close()
