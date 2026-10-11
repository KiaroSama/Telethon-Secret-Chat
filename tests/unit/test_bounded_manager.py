"""Public isolated-pause/restart seam; Wire models transport, not a crypto oracle."""

import pytest
import telethon_secret_chat as api

from .fake_client import Wire, establish

pytestmark = pytest.mark.timeout(10)


def limits(**changes):
    values = dict(records=100, bytes=8 * 1024 * 1024, age_seconds=3600)
    values.update(changes)
    return api.CustodyLimits(**values)


async def test_explicit_pause_preserves_chat_and_restart_requires_explicit_resume(tmp_path):
    wire = Wire()
    path = tmp_path / "state.json"
    manager = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    resumed = None
    try:
        chat, _ = await establish(manager, peer, wire)
        key = manager.status(chat.id).key_fingerprint
        assert len(manager.list()) == 1
        manager.pause_chat(chat.id)
        before = len(wire.a.sent)
        with pytest.raises(api.ChatPaused):
            await manager.send_message(chat.id, "refused fixture")
        assert len(wire.a.sent) == before
        assert manager.status(chat.id).key_fingerprint == key
        await manager.stop()
        resumed = api.BoundedSecretChatManager(wire.a, api.FileStorage(path), limits=limits())
        await resumed.start()
        assert resumed.pause_status(chat.id).paused
        assert resumed.status(chat.id).key_fingerprint == key
        await resumed.resume_chat(chat.id)
        await resumed.send_message(chat.id, "continued fixture")
        assert not resumed.pause_status(chat.id).paused
        assert not wire.a.discarded
        assert resumed.auto_save_secret_chats is None
    finally:
        if resumed is not None:
            await resumed.stop()
        await manager.stop()
        await peer.stop()


async def test_paused_chat_does_not_stop_another_live_chat(tmp_path):
    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        first, _ = await establish(manager, peer, wire)
        second, _ = await establish(manager, peer, wire)
        manager.pause_chat(first.id)
        await manager.send_message(second.id, "other conversation")
        await peer.send_message(second.id, "other reply")
        assert manager.pending_deliveries(second.id)[0].message.text == "other reply"
        assert manager.pause_status(first.id).paused
        assert not manager.pause_status(second.id).paused
    finally:
        await manager.stop()
        await peer.stop()


async def test_receive_refusal_keeps_prior_records_and_forbids_resume(tmp_path):
    wire = Wire()
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    allowed = {"receive": True}
    manager = api.BoundedSecretChatManager(
        wire.a,
        api.FileStorage(tmp_path / "state.json"),
        limits=limits(),
        admission=lambda chat, operation: allowed["receive"] or operation != "receive",
    )
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_message(chat.id, "held first")
        before = manager.pending_deliveries(chat.id)
        allowed["receive"] = False
        await peer.send_message(chat.id, "refused next")
        assert manager.pending_deliveries(chat.id) == before
        assert not manager.pause_status(chat.id).resumable
        with pytest.raises(api.ChatPaused):
            await manager.resume_chat(chat.id)
        assert not wire.a.discarded
    finally:
        await manager.stop()
        await peer.stop()


async def test_age_pause_preserves_custody_until_explicit_ack(monkeypatch, tmp_path):
    import time

    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits(age_seconds=10)
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_message(chat.id, "age fixture")
        before = manager.pending_deliveries(chat.id)
        now = time.time()
        monkeypatch.setattr(time, "time", lambda: now + 11)
        manager.check_custody()
        assert manager.pause_status(chat.id).paused
        assert manager.pending_deliveries(chat.id) == before
        assert not wire.a.discarded
    finally:
        await manager.stop()
        await peer.stop()


async def test_unrelated_raw_cloud_updates_do_not_consume_chat_capacity(tmp_path):
    from telethon.tl import types

    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits(chats=1)
    )
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    try:
        for number in range(3):
            update = types.UpdateNewMessage(
                message=types.Message(
                    id=number + 1,
                    peer_id=types.PeerUser(number + 100),
                    message="cloud fixture",
                    date=None,
                ),
                pts=1,
                pts_count=1,
            )
            await wire.a.deliver_update(update)
        chat = await manager.create(2000)
        assert chat.id > 0
        assert len(manager.list()) == 1
    finally:
        await manager.stop()


@pytest.mark.parametrize(
    "operation",
    [
        "accept",
        "retry_pending",
        "rekey",
        "set_ttl",
        "mark_read",
        "screenshot",
        "set_typing",
        "send_file",
        "forward_file",
    ],
)
async def test_paused_public_effects_issue_no_rpc(tmp_path, operation):
    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    try:
        chat, _ = await establish(manager, peer, wire)
        manager.pause_chat(chat.id)
        before = len(wire.a.sent), len(wire.a.uploaded)
        args = {
            "set_ttl": (3,),
            "mark_read": ([1],),
            "screenshot": ([1],),
            "send_file": (b"file",),
            "forward_file": (None,),
        }.get(operation, ())
        kwargs = {"file_name": "sample.bin"} if operation == "send_file" else {}
        with pytest.raises(api.ChatPaused):
            await getattr(manager, operation)(chat.id, *args, **kwargs)
        assert (len(wire.a.sent), len(wire.a.uploaded)) == before
    finally:
        await manager.stop()
        await peer.stop()
