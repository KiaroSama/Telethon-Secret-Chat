"""Strong checkpoint captures actual TL envelopes before any qts acknowledgment."""

import datetime
import secrets
from types import SimpleNamespace

import pytest
from telethon._updates import MessageBox, EntityCache, SessionState
from telethon.tl import types, functions

import telethon_secret_chat as api

pytestmark = pytest.mark.timeout(10)
DATE = datetime.datetime(2026, 10, 12, tzinfo=datetime.timezone.utc)


def make_box(log):
    box = MessageBox(log)
    box.load(SessionState(123, 2, False, 10, 40, int(DATE.timestamp()), 1, None), [])
    return box


async def test_ingress_commit_advances_frontier_only_after_encrypted_durable_receipt(tmp_path):
    import logging

    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    ingress = api.DurableIngress(store, limits=limits)
    box = make_box(logging.getLogger("fixture"))
    client = SimpleNamespace(
        session=SimpleNamespace(dc_id=2, auth_key=SimpleNamespace(key_id=456)),
        _mb_entity_cache=EntityCache(self_id=123),
    )
    try:
        await ingress.bind(client)
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
        await ingress.capture_live(envelope, box)
        # Capture did not mutate the caller's observed MessageBox.
        assert box.session_state()[0]["qts"] == 40
        assert ingress.checkpoint()["qts"] == 41
        assert len(ingress.pending_batches()) == 1
        await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests(
                [functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=42)]
            )
        assert ingress.pending_batches()[0].kind == "live"
    finally:
        store.close()


@pytest.mark.parametrize("source", ["live", "difference"])
async def test_real_patched_release_commit_before_implicit_ack_and_refused_raw(tmp_path, source):
    import asyncio
    from .release_harness import ReleaseLoop

    client = ReleaseLoop(source)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    task = None
    try:
        await ingress.bind(client)
        await ingress.bootstrap(client._message_box.session_state())
        client._durable_ingress = ingress
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"public-fixture-envelope",
            file=types.EncryptedFileEmpty(),
        )
        if source == "live":
            await client._updates_queue.put(
                types.Updates(
                    updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
                    users=[],
                    chats=[],
                    date=DATE,
                    seq=2,
                )
            )
        else:
            from telethon._updates.messagebox import ENTRY_ACCOUNT

            client._message_box.try_begin_get_diff(ENTRY_ACCOUNT, "fixture difference")
        task = asyncio.create_task(client._update_loop())
        client._updates_handle = task
        ready = asyncio.create_task(client.received.wait())
        done, _ = await asyncio.wait({task, ready}, return_when=asyncio.FIRST_COMPLETED)
        if task in done:
            ready.cancel()
            await asyncio.gather(ready, return_exceptions=True)
            await task
            raise AssertionError("release loop ended before durable receipt")
        await ready
        assert ingress.checkpoint()["qts"] == 41
        assert store.load_setting("durable_ingress_v1")["batches"][0]["envelope"]
        assert client.server.owed
        client.allow_ack.set()
        await client.acknowledged.wait()
        await task
        assert client.server.requests[-1].qts == 41
        assert client.server.owed == {}
        assert client.session.committed == {}  # Consumer refusal did not destroy ingress custody.
        assert ingress.pending_batches()
    finally:
        client.connected = False
        client.allow_ack.set()
        if task is not None and not task.done():
            task.cancel()
        if task is not None:
            await asyncio.gather(task, return_exceptions=True)
        store.close()


@pytest.mark.parametrize("wrapped", [False, True])
async def test_acknowledgment_request_cannot_exceed_committed_frontier(tmp_path, wrapped):
    import logging

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
        await ingress.bootstrap(make_box(logging.getLogger("fixture")).session_state())
        request = functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=41)
        if wrapped:
            request = functions.InvokeWithoutUpdatesRequest(request)
        with pytest.raises(api.IngressRefused):
            await ingress.guard_requests([request])
        await ingress.stop()
        assert (await ingress.session_state({}, []))[0]["qts"] == 40
    finally:
        store.close()


async def test_failed_ingress_commit_preserves_server_event_and_checkpoint(tmp_path):
    import asyncio
    from .release_harness import ReleaseLoop

    client = ReleaseLoop("live")
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=1, bytes=1024 * 1024, age_seconds=3600)
    )
    task = None
    try:
        await ingress.bind(client)
        await ingress.bootstrap(client._message_box.session_state())
        client._durable_ingress = ingress
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"public-fixture-envelope",
            file=types.EncryptedFileEmpty(),
        )
        first = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message=message, qts=41)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )
        await ingress.capture_live(first, client._message_box)
        # Same server-stream frontier cannot commit another whole batch within record cap.
        second = types.Updates(
            updates=[types.UpdateNewEncryptedMessage(message=message, qts=42)],
            users=[],
            chats=[],
            date=DATE,
            seq=3,
        )
        await client._updates_queue.put(second)
        task = asyncio.create_task(client._update_loop())
        await task
        assert isinstance(client._updates_error, api.IngressRefused)
        assert ingress.checkpoint()["qts"] == 41
        assert client.server.owed == {41: b"public-fixture-envelope"}
        assert client.server.requests == []
    finally:
        client.connected = False
        if task is not None and not task.done():
            task.cancel()
        if task is not None:
            await asyncio.gather(task, return_exceptions=True)
        store.close()


async def test_difference_without_per_message_qts_is_whole_batch_durable(tmp_path):
    import logging

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
        request = functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=40)
        message = types.EncryptedMessage(
            random_id=11, chat_id=7, date=DATE, bytes=b"fixture", file=types.EncryptedFileEmpty()
        )
        response = types.updates.Difference(
            new_messages=[],
            new_encrypted_messages=[message],
            other_updates=[],
            chats=[],
            users=[],
            state=types.updates.State(pts=10, qts=41, date=DATE, seq=1, unread_count=0),
        )
        await ingress.capture_difference(request, response, box)
        assert ingress.checkpoint()["qts"] == 41
        batch = ingress.pending_batches()[0]
        assert batch.envelope == bytes(response)
        from telethon_secret_chat.ingress_replay import updates_for

        encrypted = [
            item
            for item in updates_for(batch)
            if isinstance(item, types.UpdateNewEncryptedMessage)
        ]
        assert len(encrypted) == 1 and encrypted[0].qts == 0
        await ingress.capture_difference(request, response, box)
        assert len(ingress.pending_batches()) == 1
        await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=41)])
    finally:
        store.close()


async def test_real_rpc_explicit_ack_deletes_only_committed_frontier(tmp_path):
    from .release_harness import ReleaseLoop

    client = ReleaseLoop("live")
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    try:
        await ingress.bind(client)
        await ingress.bootstrap(client._message_box.session_state())
        client._durable_ingress = ingress
        with pytest.raises(api.IngressRefused):
            await client(functions.messages.ReceivedQueueRequest(max_qts=41))
        assert client.server.requests == [] and 41 in client.server.owed
        message = types.EncryptedMessage(
            random_id=11,
            chat_id=7,
            date=DATE,
            bytes=b"public-fixture-envelope",
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
            client._message_box,
        )
        await client(functions.messages.ReceivedQueueRequest(max_qts=41))
        assert client.server.requests[-1].max_qts == 41
        assert client.server.owed == {}
        assert len(ingress.pending_batches()) == 1
    finally:
        store.close()


async def test_business_qts_then_secret_qts_share_verified_nonzero_frontier(tmp_path):
    import logging
    from telethon_secret_chat.ingress_replay import updates_for

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
        cloud = types.Message(
            id=1, peer_id=types.PeerUser(99), date=DATE, message="business fixture"
        )
        business = types.UpdateBotNewBusinessMessage(
            connection_id="fixture", message=cloud, qts=41
        )
        secret = types.EncryptedMessage(
            random_id=11, chat_id=7, date=DATE, bytes=b"fixture", file=types.EncryptedFileEmpty()
        )
        envelope = types.Updates(
            updates=[business, types.UpdateNewEncryptedMessage(secret, 42)],
            users=[],
            chats=[],
            date=DATE,
            seq=2,
        )
        await ingress.capture_live(envelope, box)
        assert ingress.checkpoint()["qts"] == 42
        recovered = updates_for(ingress.pending_batches()[0])
        assert [item.qts for item in recovered] == [41, 42]
        assert isinstance(recovered[0], types.UpdateBotNewBusinessMessage)
        await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=42)])
    finally:
        store.close()


@pytest.mark.parametrize("mode", ["slice", "too_long", "gap"])
async def test_difference_slices_and_impossible_gap_do_not_invent_frontier(tmp_path, mode):
    import logging
    from telethon_secret_chat.ingress import load_box

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
        request = functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=40)
        if mode == "too_long":
            with pytest.raises(api.IngressRefused):
                await ingress.capture_difference(
                    request, types.updates.DifferenceTooLong(pts=11), box
                )
            with pytest.raises(api.IngressRefused):
                await ingress.guard_requests([request])
            assert ingress.checkpoint()["qts"] == 40
        elif mode == "gap":
            message = types.EncryptedMessage(
                random_id=12,
                chat_id=7,
                date=DATE,
                bytes=b"gap fixture",
                file=types.EncryptedFileEmpty(),
            )
            await ingress.capture_live(
                types.Updates(
                    updates=[types.UpdateNewEncryptedMessage(message, 42)],
                    users=[],
                    chats=[],
                    date=DATE,
                    seq=2,
                ),
                box,
            )
            assert ingress.checkpoint()["qts"] == 40
            with pytest.raises(api.IngressRefused):
                await ingress.guard_requests([functions.messages.ReceivedQueueRequest(max_qts=42)])
            assert len(ingress.pending_batches()) == 1
        else:
            for qts, final in ((41, False), (42, True)):
                state = types.updates.State(pts=10, qts=qts, date=DATE, seq=1, unread_count=0)
                params = dict(
                    new_messages=[],
                    new_encrypted_messages=[],
                    other_updates=[],
                    chats=[],
                    users=[],
                )
                response = (
                    types.updates.Difference(state=state, **params)
                    if final
                    else types.updates.DifferenceSlice(intermediate_state=state, **params)
                )
                await ingress.capture_difference(request, response, box)
                assert ingress.checkpoint()["qts"] == qts
                load_box(box, ingress._state["checkpoint"])
                request = functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=qts)
            assert [item.after["session"]["qts"] for item in ingress.pending_batches()] == [41, 42]
    finally:
        store.close()


async def test_caller_difference_rpc_then_live_envelope_keeps_observed_and_durable_qts(tmp_path):
    from .release_harness import ReleaseLoop

    client = ReleaseLoop("difference")
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    store = api.EncryptedIngressStorage(
        tmp_path / "state.bin", key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
    )
    ingress = api.DurableIngress(
        store, limits=api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    )
    try:
        await ingress.bind(client)
        await ingress.bootstrap(client._message_box.session_state())
        client._durable_ingress = ingress
        response = await client(functions.updates.GetDifferenceRequest(pts=10, date=DATE, qts=40))
        assert response.state.qts == 41
        assert ingress.checkpoint()["qts"] == 41
        assert client._message_box.session_state()[0]["qts"] == 41
        message = types.EncryptedMessage(
            random_id=12,
            chat_id=7,
            date=DATE,
            bytes=b"next fixture",
            file=types.EncryptedFileEmpty(),
        )
        await ingress.capture_live(
            types.Updates(
                updates=[types.UpdateNewEncryptedMessage(message, 42)],
                users=[],
                chats=[],
                date=DATE,
                seq=2,
            ),
            client._message_box,
        )
        assert ingress.checkpoint()["qts"] == 42
        assert len(ingress.pending_batches()) == 2
    finally:
        store.close()
