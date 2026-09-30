"""Restore, media and typed-wire boundaries exercised by the September 30 audit."""

import asyncio
from dataclasses import replace
from types import SimpleNamespace

import pytest
from telethon.extensions import BinaryReader

from telethon_secret_chat import files, framing, sequence
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.events import ChatReady
from telethon_secret_chat.media import MediaReference
from telethon_secret_chat.schema import secret_tl as tl

from .helpers import peer_message, ready_manager


def reference():
    key, iv = files.new_file_key()
    document = tl.DecryptedMessageMediaDocument(
        thumb=b"",
        thumb_w=0,
        thumb_h=0,
        mime_type="application/octet-stream",
        size=16,
        key=key,
        iv=iv,
        attributes=[],
        caption="",
    )
    return MediaReference(
        chat_id=7,
        file_id=8,
        access_hash=9,
        dc_id=1,
        key_fingerprint=files.file_fingerprint(key, iv),
        media=bytes(document),
    )


@pytest.mark.parametrize("as_reference", [True, False])
async def test_forward_rejects_mismatched_file_keys_before_reserving(as_reference):
    manager, chat = ready_manager()
    ref = reference()
    damaged = replace(ref, key_fingerprint=ref.key_fingerprint ^ 1)
    source = (
        damaged
        if as_reference
        else SimpleNamespace(
            chat_id=7,
            media=damaged.decoded(),
            file=damaged.encrypted_file(),
        )
    )
    before = manager._storage.load(7)
    with pytest.raises(MessageRejected):
        await manager.forward_file(7, source)
    assert manager._storage.load(7) == before
    assert manager._client.sent == []


@pytest.mark.parametrize("kind", ["sticker", "video_note"])
async def test_forward_preserves_the_captionless_kind_contract(kind):
    manager, _ = ready_manager()
    ref = reference()
    doc = ref.decoded()
    doc.attributes = files.attributes_for(kind, "item.bin")
    source = replace(ref, media=bytes(doc))
    with pytest.raises(ValueError, match="caption"):
        await manager.forward_file(7, source, caption="must not be silently discarded")
    assert manager._client.sent == []


@pytest.mark.parametrize(
    "change", ["negative-size", "trailing", "bool-version", "bad-fingerprint"]
)
def test_media_reference_rejects_unusable_persisted_data(change):
    ref = reference()
    data = ref.to_dict()
    if change == "negative-size":
        doc = ref.decoded()
        doc.size = -1
        data["media"] = bytes(doc).hex()
    elif change == "trailing":
        data["media"] += "00000000"
    elif change == "bool-version":
        data["version"] = True
    else:
        data["key_fingerprint"] ^= 1
    with pytest.raises(ValueError):
        MediaReference.from_dict(data)


@pytest.mark.parametrize("field", ["media", "entities", "action", "attribute", "stickerset"])
def test_a_known_constructor_in_the_wrong_tl_family_is_not_a_valid_message(field):
    wrapper = peer_message(0)
    wrong = tl.DecryptedMessageActionNoop()
    if field == "media":
        original = tl.DecryptedMessageMediaEmpty()
        wrapper.message.media = original
    elif field == "entities":
        original = tl.MessageEntityBold(offset=0, length=1)
        wrapper.message.entities = [original]
    elif field == "action":
        original = tl.DecryptedMessageActionNoop()
        wrapper.message = tl.DecryptedMessageService(random_id=1, action=original)
        wrong = tl.DecryptedMessageMediaEmpty()
    else:
        ref = reference()
        doc = ref.decoded()
        if field == "attribute":
            original = tl.DocumentAttributeFilename(file_name="x")
            doc.attributes = [original]
        else:
            original = tl.InputStickerSetEmpty()
            doc.attributes = [tl.DocumentAttributeSticker(alt="", stickerset=original)]
        wrapper.message.media = doc
    body = bytes(wrapper)
    assert body.count(bytes(original)) == 1
    malformed = body.replace(bytes(original), bytes(wrong))
    with pytest.raises(MessageRejected, match="parsed") as caught:
        framing.unwrap(malformed, chat_id=7)
    assert caught.value.fatal


async def test_failed_service_effect_does_not_publish_or_change_live_state(monkeypatch):
    manager, chat = ready_manager()
    wrapper = peer_message(0)
    wrapper.message = tl.DecryptedMessageService(
        random_id=99,
        action=tl.DecryptedMessageActionSetMessageTTL(ttl_seconds=9),
    )
    chat.pending_deliveries = [sequence.pack(wrapper)]
    manager._save(chat)
    reported = []
    manager.on("ServiceActionReceived", reported.append)

    def fail():
        raise OSError("synthetic storage failure")

    monkeypatch.setattr(manager._storage, "_write", fail)
    with pytest.raises(OSError):
        await manager._drain_deliveries(chat)
    assert chat.ttl == manager._storage.load(7)["ttl"] == 0
    assert len(chat.pending_deliveries) == 1
    assert reported == []


async def test_a_future_returned_by_a_handler_during_shutdown_is_cancelled():
    manager, _ = ready_manager()
    future = asyncio.get_running_loop().create_future()
    manager.on("ChatReady", lambda event: future)
    manager._stopping = True
    manager._emit(ChatReady(7, 9, 0))
    assert future.cancelled()


def test_reference_decode_has_one_complete_file_media_object():
    ref = reference()
    with BinaryReader(ref.media) as reader:
        decoded = tl.read_object(reader)
    assert decoded.RESULT_TYPE == "DecryptedMessageMedia"
    assert MediaReference.from_dict(ref.to_dict()).decoded() == decoded
