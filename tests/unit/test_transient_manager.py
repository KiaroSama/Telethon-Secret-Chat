"""Real protocol/storage seams, external network replaced by the existing Wire."""

from types import SimpleNamespace

import pytest

from telethon_secret_chat import MemoryStorage, SecretChatManager

from .fake_client import Wire, establish

pytestmark = pytest.mark.timeout(10)


def authorize(client):
    client.session = SimpleNamespace(
        auth_key=SimpleNamespace(key_id=1234 + client.user_id), dc_id=2
    )
    client.is_connected = lambda: True

    async def get_me():
        return SimpleNamespace(id=client.user_id, bot=False)

    client.get_me = get_me


async def test_transient_send_receive_never_reaches_persistent_state(tmp_path):
    from telethon_secret_chat import ProtectedFileStorage, TransientSecretChatManager

    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    received = []

    async def collect(event):
        received.append(event)

    a.on("MessageReceived", collect)
    await a.start()
    await b.start()
    try:
        chat_a, _ = await establish(a, b, wire)
        sentinel = "synthetic private text not to persist"
        await a.send_message(chat_a.id, sentinel)
        await b.send_message(chat_a.id, sentinel)
        await a.settle()
        assert len(received) == 1
        assert bool(received[0].text == sentinel)
        raw = store.path.read_bytes()
        assert bool(sentinel.encode("utf-8") not in raw)
        assert a._client is wire.a
        assert not a.is_suspended(chat_a.id)
    finally:
        await a.stop()
        await b.stop()
    assert a.is_suspended(chat_a.id)
    assert not wire.a.discarded


async def test_transient_media_download_has_no_disk_intermediate(tmp_path):
    from telethon_secret_chat import ProtectedFileStorage, TransientSecretChatManager

    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    received = []

    async def collect(event):
        received.append(event)

    a.on("MessageReceived", collect)
    await a.start()
    await b.start()
    try:
        chat, _ = await establish(a, b, wire)
        content = b"synthetic file body" * 4
        await b.send_file(chat.id, content, file_name="sample.bin")
        await a.settle()
        assert len(received) == 1
        before = {path.name for path in tmp_path.iterdir()}
        result = await a.receive_file(received[0])
        assert bool(result == content)
        assert {path.name for path in tmp_path.iterdir()} == before
        raw = store.path.read_bytes()
        media = received[0].media
        assert bool(media.key.hex().encode("ascii") not in raw)
        assert bool(media.iv.hex().encode("ascii") not in raw)
        assert wire.a.download_targets == ["BoundedDownload"]
    finally:
        await a.stop()
        await b.stop()


async def test_every_atomic_file_snapshot_and_log_excludes_ordinary_material(
    tmp_path, monkeypatch, caplog
):
    from telethon.tl import types
    from telethon_secret_chat import ProtectedFileStorage, TransientSecretChatManager

    snapshots = []
    replace = __import__("os").replace

    def capture(source, target):
        if str(target) == str(tmp_path / "protected.json"):
            from pathlib import Path

            snapshots.append(Path(source).read_bytes())
        return replace(source, target)

    monkeypatch.setattr("telethon_secret_chat.storage.os.replace", capture)
    wire = Wire()
    authorize(wire.a)
    store = ProtectedFileStorage(tmp_path / "protected.json")
    a = TransientSecretChatManager(wire.a, store)
    b = SecretChatManager(wire.b, MemoryStorage())
    events = []

    async def collect(event):
        events.append(event)

    a.on("MessageReceived", collect)
    await a.start()
    await b.start()
    caplog.set_level("DEBUG", logger="telethon_secret_chat")
    try:
        chat, _ = await establish(a, b, wire)
        text = "synthetic snapshot content"
        entities = [
            types.MessageEntityTextUrl(offset=0, length=4, url="https://example.invalid/sentinel")
        ]
        await a.send_message(chat.id, text, entities=entities)
        await b.send_file(chat.id, b"synthetic media", file_name="snapshot.bin")
        await a.settle()
        sent = [
            request for request in wire.a.sent if type(request).__name__ == "SendEncryptedRequest"
        ]
        ordinary = [{"frame": request.data.hex()} for request in sent]
        assert ordinary
        tokens = [
            text.encode("utf-8"),
            b"example.invalid/sentinel",
            ordinary[0]["frame"].encode("ascii"),
            events[0].media.key.hex().encode("ascii"),
            events[0].media.iv.hex().encode("ascii"),
        ]
        await a.stop()
        assert snapshots
        for snapshot in snapshots:
            assert not any(token in snapshot for token in tokens)
            state = __import__("json").loads(snapshot)
            assert state["out"] == state["in"] == {}
            assert all(
                "pending_deliveries" not in item["chat"] for item in state["chats"].values()
            )
        logs = caplog.text.encode("utf-8")
        assert not any(token in logs for token in tokens)
        backup = tmp_path / "backup.json"
        backup.write_bytes(store.path.read_bytes())
        assert not any(token in backup.read_bytes() for token in tokens)
        assert not list(tmp_path.glob(".secret-chat-store-*"))
    finally:
        await a.stop()
        await b.stop()
