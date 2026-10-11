"""Capped memory receipt on the durable manager, with no local media files."""

import pytest
import telethon_secret_chat as api

from .fake_client import Wire, establish
from .test_bounded_manager import limits

pytestmark = pytest.mark.timeout(10)


async def test_receive_file_is_capped_memory_only_without_auto_save(tmp_path):
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
        content = b"bounded media fixture\x00" * 100
        await peer.send_file(chat.id, content, file_name="sample.bin")
        message = manager.pending_deliveries(chat.id)[0].message
        before = set(tmp_path.iterdir())
        with pytest.raises(api.ChatPaused):
            await manager.receive_file(message, max_bytes=len(content) - 1)
        assert wire.a.download_targets == []
        assert await manager.receive_file(message, max_bytes=len(content)) == content
        assert set(tmp_path.iterdir()) == before
        assert manager.auto_save_secret_chats is None
    finally:
        await manager.stop()
        await peer.stop()


@pytest.mark.parametrize("failure", ["oversize", "short", "cancel"])
async def test_download_failure_closes_ram_buffer_and_creates_no_file(tmp_path, failure):
    import asyncio

    wire = Wire()
    manager = api.BoundedSecretChatManager(
        wire.a, api.FileStorage(tmp_path / "state.json"), limits=limits()
    )
    peer = api.SecretChatManager(wire.b, api.MemoryStorage())
    await manager.stop_auto_save_secret_chats()
    await manager.start()
    await peer.start()
    buffers = []
    try:
        chat, _ = await establish(manager, peer, wire)
        await peer.send_file(chat.id, b"fixture media", file_name="sample.bin")
        message = manager.pending_deliveries(chat.id)[0].message
        before = set(tmp_path.iterdir())

        async def download(location, out, **kwargs):
            buffers.append(out)
            if failure == "cancel":
                raise asyncio.CancelledError
            out.write(b"x" * (32 if failure == "oversize" else 1))

        wire.a.download_file = download
        expected = asyncio.CancelledError if failure == "cancel" else api.ChatPaused
        with pytest.raises(expected):
            await manager.receive_file(message, max_bytes=1024)
        assert buffers and buffers[0].closed
        assert set(tmp_path.iterdir()) == before
    finally:
        await manager.stop()
        await peer.stop()
