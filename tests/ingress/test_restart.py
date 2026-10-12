"""Real protected checkpoint reconstruction preserves owed ingress identities."""

import logging
import secrets
from types import SimpleNamespace

import pytest
from telethon._updates import EntityCache
from telethon.tl import types, functions

import telethon_secret_chat as api
from .test_frontier import DATE, make_box

pytestmark = pytest.mark.timeout(10)


async def test_committed_ingress_reconstructs_after_ack_before_application(tmp_path):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "state.bin"
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    ingress = api.DurableIngress(store, limits=limits)
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"public fixture",
            file=types.EncryptedFileEmpty(),
        )
        envelope = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )
        await ingress.capture_live(envelope, box)
        await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
        identity = ingress.pending_batches()[0].identity
    finally:
        store.close()
    restored_store = api.EncryptedIngressStorage(
        path, key=key, binding=binding, max_bytes=1024 * 1024
    )
    try:
        restored = api.DurableIngress(restored_store, limits=limits)
        await restored.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await restored.restore(box)
        assert box.session_state()[0]["qts"] == 41
        assert restored.pending_batches()[0].identity == identity
        assert restored.pending_batches()[0].cursor == 0
    finally:
        restored_store.close()


async def test_corrupt_declared_frontier_cannot_authorize_acknowledgment(tmp_path):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=key, binding=binding, max_bytes=1024 * 1024
    )
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        ingress = api.DurableIngress(store, limits=limits)
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        message = types.EncryptedMessage(
            random_id=11, chat_id=7, date=DATE, bytes=b"fixture", file=types.EncryptedFileEmpty()
        )
        await ingress.capture_live(
            types.Updates(
                updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
                users=[],
                chats=[],
                date=DATE,
                seq=2,
            ),
            box,
        )
        corrupted = store.load_setting("durable_ingress_v1")
        corrupted["batches"][0]["after"]["session"]["qts"] = 42
        corrupted["checkpoint"]["session"]["qts"] = 42
        store.save_setting("durable_ingress_v1", corrupted)
        with pytest.raises(api.IngressRefused):
            api.DurableIngress(store, limits=limits)
    finally:
        store.close()


@pytest.mark.parametrize("boundary", ["before_replace", "after_replace"])
async def test_uncertain_disk_commit_never_authorizes_ack_and_reopens_exact_state(
    tmp_path, monkeypatch, boundary
):
    import os

    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "state.bin"
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    ingress = api.DurableIngress(store, limits=limits)
    real_replace = os.replace
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        message = types.EncryptedMessage(
            random_id=11, chat_id=7, date=DATE, bytes=b"fixture", file=types.EncryptedFileEmpty()
        )
        envelope = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )

        def fail_replace(source, destination):
            if boundary == "after_replace":
                real_replace(source, destination)
            raise OSError("synthetic commit barrier")

        monkeypatch.setattr(os, "replace", fail_replace)
        with pytest.raises(api.IngressRefused):
            await ingress.capture_live(envelope, box)
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
        assert box.session_state()[0]["qts"] == 40
    finally:
        monkeypatch.setattr(os, "replace", real_replace)
        store.close()
    restored = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    try:
        recovered = api.DurableIngress(restored, limits=limits)
        await recovered.bind(client)
        assert recovered.checkpoint()["qts"] == (41 if boundary == "after_replace" else 40)
        assert len(recovered.pending_batches()) == (1 if boundary == "after_replace" else 0)
    finally:
        restored.close()


async def test_difference_rpc_handoff_remains_valid_after_committed_batch_is_consumed(tmp_path):

    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=key, binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        request = functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=40)
        response = types.updates.DifferenceEmpty(date=DATE, seq=1)
        await ingress.guard_requests([request])
        await ingress.guard_result(request, response, box)
        # Simulate committed consumer completion between RPC receipt and loop application.
        state = store.load_setting("durable_ingress_v1")
        state["batches"] = []
        store.save_setting("durable_ingress_v1", state)
        ingress._state = state
        await ingress.capture_difference(request, response, box)
        assert ingress.pending_batches() == []
        assert ingress.checkpoint()["qts"] == 40
    finally:
        store.close()


async def test_session_projection_never_persists_uncommitted_observed_frontier(tmp_path):
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        observed, channels = box.session_state()
        observed["qts"] = 999
        committed, _ = await ingress.session_state(observed, channels)
        assert committed["qts"] == 40
        await ingress.stop()
        assert (await ingress.session_state(observed, channels))[0]["qts"] == 40
    finally:
        store.close()


async def test_cancelled_lock_wait_cannot_commit_or_acknowledge_new_ingress(tmp_path):
    import asyncio

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    task = None
    try:
        await ingress.bind(client)
        box = make_box(logging.getLogger("fixture"))
        await ingress.bootstrap(box.session_state())
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"cancel fixture",
            file=types.EncryptedFileEmpty(),
        )
        envelope = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message, 41)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )
        await ingress._lock.acquire()
        task = asyncio.create_task(ingress.capture_live(envelope, box))
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        ingress._lock.release()
        assert ingress.pending_batches() == [] and ingress.checkpoint()["qts"] == 40
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
    finally:
        if ingress._lock.locked():
            ingress._lock.release()
        if task is not None:
            task.cancel()
            await asyncio.gather(task, return_exceptions=True)
        store.close()
