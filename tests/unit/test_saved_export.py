"""Saved export is offline and preserves its source even after the chat is gone."""

import json
from pathlib import Path
from datetime import timezone

import pytest
from telethon.tl import types

from telethon_secret_chat import MemoryStorage, SecretChatManager

SELF = types.User(id=1, first_name="Me", is_self=True)
PEER = types.User(id=2, first_name="Peer")
BASE = 1_700_000_000


@pytest.fixture
async def saved(tmp_path):
    manager = SecretChatManager(object(), MemoryStorage())
    folder = tmp_path / "saved source"
    await manager.start_auto_save_secret_chats(folder)
    chat = folder / "7"
    chat.mkdir(parents=True)
    records = [
        {
            "v": 1,
            "type": "message",
            "id": 99,
            "date": BASE,
            "out": True,
            "text": "hello فارسی",
            "entities": [],
            "media": None,
        },
        {
            "v": 1,
            "type": "message",
            "id": 100,
            "date": BASE + 100,
            "out": False,
            "text": "reply",
            "entities": [],
            "reply_to": 99,
            "media": None,
        },
    ]
    source = chat / "messages.jsonl"
    source.write_text(
        "\n".join(json.dumps(r, ensure_ascii=False) for r in records) + "\n", encoding="utf-8"
    )
    await manager.stop_auto_save_secret_chats()
    return manager, source


@pytest.mark.parametrize("format", ["html", "json", "both"])
async def test_saved_copy_exports_offline_after_chat_removal(saved, tmp_path, format):
    manager, source = saved
    before = source.read_bytes()
    result = await manager.export_saved_messages(
        7, tmp_path / "export destination", self_user=SELF, peer_user=PEER, format=format
    )
    output = Path(result.path)
    assert (result.messages, result.files, result.takeout) == (2, 0, False)
    if format in ("json", "both"):
        data = json.loads((output / "result.json").read_text(encoding="utf-8"))
        assert [m["id"] for m in data["messages"]] == [1, 2]
        assert data["messages"][0]["text"] == "hello فارسی"
        assert data["messages"][1]["reply_to_message_id"] == 1
    if format in ("html", "both"):
        page = (output / "messages.html").read_text(encoding="utf-8")
        assert "hello فارسی" in page and "message2" in page
        assert (output / "css" / "style.css").is_file()
        assert (output / "js" / "script.js").is_file()
    assert source.read_bytes() == before
    assert manager.list() == []


@pytest.mark.parametrize(
    "options",
    [
        {"format": "bad"},
        {"size_limit": -1},
        {"size_limit": True},
        {"date_from": -1},
        {"date_till": 2, "date_from": 3},
        {"media_types": ["unknown"]},
        {"media_types": "photo"},
    ],
)
async def test_invalid_options_write_nothing(saved, tmp_path, options):
    manager, source = saved
    destination = tmp_path / "invalid"
    before = source.read_bytes()
    with pytest.raises(ValueError):
        await manager.export_saved_messages(
            7, destination, self_user=SELF, peer_user=PEER, **options
        )
    assert not destination.exists()
    assert source.read_bytes() == before


async def test_export_cannot_overlap_saved_source(saved):
    manager, source = saved
    before = source.read_bytes()
    with pytest.raises(ValueError, match="overlap"):
        await manager.export_saved_messages(7, source.parent, self_user=SELF, peer_user=PEER)
    assert source.read_bytes() == before


async def test_export_failure_preserves_source_and_hides_contents(
    saved, tmp_path, monkeypatch, caplog
):
    from telethon_secret_chat import saved_export

    manager, source = saved
    before = source.read_bytes()

    def refuse(*args):
        raise OSError("private message and credentials")

    monkeypatch.setattr(saved_export, "export_saved_chat", refuse)
    with pytest.raises(OSError, match="Saved export failed") as failure:
        await manager.export_saved_messages(
            7, tmp_path / "failure", self_user=SELF, peer_user=PEER
        )
    assert "private message" not in str(failure.value) + caplog.text
    assert source.read_bytes() == before


async def test_file_copy_failure_closes_output_handles(saved, tmp_path, monkeypatch):
    from telethon_secret_chat.tdexport import secret_saved
    from telethon_secret_chat.tdexport.output_file import File

    manager, source = saved
    file = source.parent / "photo.jpg"
    file.write_bytes(b"synthetic copy failure input")
    records = manager.read_saved_messages(7)
    records[0]["media"] = {
        "_": "decryptedMessageMediaPhoto",
        "w": 1,
        "h": 1,
        "size": file.stat().st_size,
    }
    records[0]["file"] = str(file)
    source.write_text("\n".join(json.dumps(r) for r in records) + "\n", encoding="utf-8")
    opened = []
    reopen = File._reopen

    def track(self):
        reopen(self)
        if self._file is not None:
            opened.append(self._file)

    def refuse(*args):
        raise OSError("copy refused")

    monkeypatch.setattr(File, "_reopen", track)
    monkeypatch.setattr(secret_saved.shutil, "copyfile", refuse)
    with pytest.raises(OSError, match="Saved export failed"):
        await manager.export_saved_messages(
            7, tmp_path / "output", self_user=SELF, peer_user=PEER, format="both"
        )
    assert opened and all(handle.closed for handle in opened)
    assert file.read_bytes() == b"synthetic copy failure input"


async def test_date_range_reaches_desktop_writer(saved, tmp_path, monkeypatch):
    from telethon_secret_chat.tdexport import model_format

    monkeypatch.setattr(model_format, "LOCAL_TIMEZONE", timezone.utc)
    manager, _ = saved
    result = await manager.export_saved_messages(
        7,
        tmp_path / "limited",
        self_user=SELF,
        peer_user=PEER,
        format="json",
        date_from=BASE + 50,
        date_till=BASE + 150,
    )
    data = json.loads((Path(result.path) / "result.json").read_text(encoding="utf-8"))
    assert [m["id"] for m in data["messages"]] == [2]
