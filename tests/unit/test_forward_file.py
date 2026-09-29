"""Forwarding a received file without downloading or uploading it (spec 005, US3).

Telegram's end-to-end guide: files "can be forwarded to other secret chats using the constructor
inputEncryptedFile"; TDLib re-sends the ORIGINAL key and iv (ADR 0006). Verified cross-chat on the
official Android client 2026-09-29.
"""

import os

import pytest
from telethon.tl import types

from telethon_secret_chat import MediaReference
from telethon_secret_chat.errors import MessageRejected

from .fake_client import establish


@pytest.fixture
async def two_chats(pair, tmp_path):
    """B received a file from A in chat 1; A and B also share chat 2."""
    wire, a, b = pair
    chat1_a, chat1_b = await establish(a, b, wire)
    chat2_a, chat2_b = await establish(a, b, wire)
    got_b, got_a = [], []
    b.on("MessageReceived", got_b.append)
    a.on("MessageReceived", got_a.append)
    content = os.urandom(4000)
    path = tmp_path / "clip.mp4"
    path.write_bytes(content)
    await a.send_file(
        chat1_a.id,
        path,
        caption="orig",
        duration=3,
        width=320,
        height=240,
        thumbnail=b"\xff\xd8\xff\xe0" + b"\x00" * 20,
        thumbnail_size=(90, 68),
    )
    return wire, a, b, chat1_b, chat2_b, got_a, got_b, content


@pytest.mark.parametrize("as_reference", [False, True])
async def test_a_forward_reuses_the_file_and_moves_no_bytes(two_chats, tmp_path, as_reference):
    wire, a, b, _, chat2_b, got_a, got_b, content = two_chats
    source = got_b[0]
    if as_reference:
        source = MediaReference.from_dict(source.media_reference.to_dict())
    uploads, downloads = len(wire.b.uploaded), len(wire.b.download_targets)
    await b.forward_file(chat2_b.id, source, caption="fwd", reply_to=42)
    assert (len(wire.b.uploaded), len(wire.b.download_targets)) == (uploads, downloads)
    handle = wire.b.sent_files[-1]
    original = got_b[0]
    assert isinstance(handle, types.InputEncryptedFile)
    assert (handle.id, handle.access_hash) == (original.file.id, original.file.access_hash)
    arrived = got_a[-1]
    assert arrived.chat_id != original.chat_id
    assert (arrived.text, arrived.reply_to) == ("fwd", 42)
    for name in ("key", "iv", "size", "thumb", "thumb_w", "thumb_h", "mime_type"):
        assert getattr(arrived.media, name) == getattr(original.media, name), name
    assert repr(arrived.media.attributes) == repr(original.media.attributes)
    assert (await a.save_file(arrived, tmp_path / "fwd.mp4")).read_bytes() == content


async def test_a_forward_is_retained_with_its_handle(two_chats):
    wire, _, b, _, chat2_b, _, got_b, _ = two_chats
    await b.forward_file(chat2_b.id, got_b[0])
    retained = b._storage.retained_out(chat2_b.id)
    assert retained and retained[-1].get("method") == "file"
    stored = bytes.fromhex(retained[-1]["file"])
    assert stored == bytes(
        types.InputEncryptedFile(id=got_b[0].file.id, access_hash=got_b[0].file.access_hash)
    )


async def test_a_photo_is_forwarded_as_a_photo(pair, tmp_path):
    from telethon_secret_chat.schema import secret_tl as tl

    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    photo = tl.DecryptedMessageMediaPhoto(
        thumb=b"",
        thumb_w=0,
        thumb_h=0,
        w=10,
        h=10,
        size=16,
        key=os.urandom(32),
        iv=os.urandom(32),
        caption="",
    )
    from telethon_secret_chat import files

    key, iv = photo.key, photo.iv
    wire.a.stored[77] = files.encrypt_file(b"p" * 16, key, iv)
    wire.b.stored[77] = wire.a.stored[77]
    wire.a.fingerprints[77] = wire.b.fingerprints[77] = files.file_fingerprint(key, iv)
    fake = type("Received", (), {})()
    fake.chat_id, fake.media = 1, photo
    fake.file = types.EncryptedFile(
        id=77, access_hash=0, size=16, dc_id=1, key_fingerprint=files.file_fingerprint(key, iv)
    )
    await a.forward_file(chat_a.id, fake)
    assert isinstance(got[-1].media, tl.DecryptedMessageMediaPhoto)


async def test_a_message_without_a_file_is_refused_before_sending(two_chats):
    wire, a, b, chat1_b, chat2_b, _, got_b, _ = two_chats
    await a.send_message(chat1_b.id, "plain")
    text_only = got_b[-1]
    assert text_only.media is None
    sent_before = len(wire.b.sent_files)
    with pytest.raises(MessageRejected):
        await b.forward_file(chat2_b.id, text_only)
    assert len(wire.b.sent_files) == sent_before


async def test_a_huge_file_needs_the_peer_at_layer_143(two_chats):
    _, _, b, _, chat2_b, _, got_b, _ = two_chats
    ref = got_b[0].media_reference
    media = ref.decoded()
    media.size = 2**31 + 1
    huge = MediaReference(**{**vars(ref), "media": bytes(media)})
    b._entity(chat2_b.id).layer = 101
    with pytest.raises(ValueError):
        await b.forward_file(chat2_b.id, huge)


async def test_a_forward_to_an_older_peer_uses_the_older_document_shape(two_chats, tmp_path):
    from telethon_secret_chat.schema import secret_tl as tl

    wire, a, b, _, chat2_b, got_a, got_b, content = two_chats
    b._entity(chat2_b.id).layer = 101
    await b.forward_file(chat2_b.id, got_b[0])
    arrived = got_a[-1]
    assert isinstance(arrived.media, tl.DecryptedMessageMediaDocument_7afe8ae2)
    assert (await a.save_file(arrived, tmp_path / "old.mp4")).read_bytes() == content
