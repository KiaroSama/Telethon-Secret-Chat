"""The eight media kinds, and the attributes each one puts on the wire.

``decryptedMessageMediaDocument`` carries ``attributes:Vector<DocumentAttribute>``,
and that vector is the ONLY thing that tells a receiving client an ``.ogg`` is a
voice note rather than a file to download. Sending every file with nothing but
``documentAttributeFilename`` is what makes audio arrive as a FILE and a round video
arrive as a FILE - the bytes are right, the kind is lost.

The eight names are the ones the MCP tools already speak, so the two agree by
construction: photo, video, document, audio, animation, sticker, video_note,
voice_note. Two of them carry no caption - ``sticker`` and ``video_note`` - because
the protocol gives them no caption field, and a caption offered with one is refused
rather than dropped silently.

A kind the file cannot be is refused BEFORE the upload, for the same reason §6
refuses a bad fingerprint before the write: the failure that costs nothing is the
one that happens before the bytes move.
"""

import os

import pytest

from telethon_secret_chat import files
from telethon_secret_chat.schema import secret_tl as tl


@pytest.fixture
async def pair():
    from telethon_secret_chat import SecretChatManager
    from telethon_secret_chat.storage import MemoryStorage

    from .fake_client import Wire

    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    yield wire, a, b
    await a.stop()
    await b.stop()


@pytest.fixture
async def chat(pair):
    """Two established managers, plus a sender that returns what arrived."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)

    async def send(tmp_path, name, **kwargs):
        source = tmp_path / name
        source.write_bytes(os.urandom(64))
        await a.send_file(chat_a.id, source, **kwargs)
        return got[-1]

    return wire, a, chat_a, send


def of_type(message, attribute_type):
    found = [a for a in message.media.attributes if isinstance(a, attribute_type)]
    assert (
        found
    ), f"no {attribute_type.TL_NAME} among {[a.TL_NAME for a in message.media.attributes]}"
    return found[0]


# --- the vocabulary -----------------------------------------------------------


def test_the_package_names_the_eight_kinds():
    """The MCP tools' own eight, so the migration is not a downgrade to one."""
    assert files.MEDIA_KINDS == (
        "photo",
        "video",
        "document",
        "audio",
        "animation",
        "sticker",
        "video_note",
        "voice_note",
    )


def test_two_kinds_carry_no_caption():
    """``decryptedMessageMediaDocument`` has a caption field, but a sticker and a
    round video are not captioned by any client: the protocol offers the sender no
    place to put one that a receiver would show."""
    assert files.CAPTIONLESS_KINDS == frozenset({"sticker", "video_note"})


# --- what each kind attaches --------------------------------------------------


async def test_a_document_carries_only_its_filename(chat, tmp_path):
    _, _, _, send = chat
    message = await send(tmp_path, "report.txt")
    assert [a.TL_NAME for a in message.media.attributes] == ["documentAttributeFilename"]


async def test_audio_carries_the_audio_attribute(chat, tmp_path):
    """``documentAttributeAudio#9852f9c6``, with ``voice`` unset - a song, not a
    voice note."""
    _, _, _, send = chat
    attribute = of_type(await send(tmp_path, "song.mp3"), tl.DocumentAttributeAudio)
    assert not attribute.voice


async def test_a_voice_note_sets_the_voice_flag(chat, tmp_path):
    """``voice:flags.10?true``. Without it the same ``.ogg`` is a file to download."""
    _, _, _, send = chat
    attribute = of_type(
        await send(tmp_path, "note.ogg", kind="voice_note"), tl.DocumentAttributeAudio
    )
    assert attribute.voice


async def test_video_carries_the_video_attribute(chat, tmp_path):
    """``documentAttributeVideo#ef02ce6``, ``round_message`` unset."""
    _, _, _, send = chat
    attribute = of_type(await send(tmp_path, "clip.mp4"), tl.DocumentAttributeVideo)
    assert not attribute.round_message


async def test_a_video_note_sets_round_message(chat, tmp_path):
    """``round_message:flags.0?true`` - the difference between a video and the round
    bubble Telegram calls a video message."""
    _, _, _, send = chat
    attribute = of_type(
        await send(tmp_path, "round.mp4", kind="video_note"), tl.DocumentAttributeVideo
    )
    assert attribute.round_message


async def test_a_photo_carries_its_image_size(chat, tmp_path):
    _, _, _, send = chat
    of_type(await send(tmp_path, "picture.jpg"), tl.DocumentAttributeImageSize)


async def test_an_animation_is_marked_animated(chat, tmp_path):
    """A GIF is an animation to every Telegram client; ``documentAttributeAnimated``
    is what says so."""
    _, _, _, send = chat
    of_type(await send(tmp_path, "loop.gif"), tl.DocumentAttributeAnimated)


async def test_a_sticker_carries_the_sticker_attribute(chat, tmp_path):
    _, _, _, send = chat
    of_type(await send(tmp_path, "face.webp", kind="sticker"), tl.DocumentAttributeSticker)


async def test_an_explicit_document_kind_beats_the_guess(chat, tmp_path):
    """The caller who says ``document`` about an ``.mp3`` means the file, not the
    player."""
    _, _, _, send = chat
    message = await send(tmp_path, "song.mp3", kind="document")
    assert [a.TL_NAME for a in message.media.attributes] == ["documentAttributeFilename"]


async def test_every_kind_keeps_the_filename(chat, tmp_path):
    """The kind is added to the filename, never instead of it."""
    _, _, _, send = chat
    for name, kind in [
        ("a.mp3", "audio"),
        ("b.ogg", "voice_note"),
        ("c.mp4", "video"),
        ("d.mp4", "video_note"),
        ("e.jpg", "photo"),
        ("f.gif", "animation"),
        ("g.webp", "sticker"),
        ("h.bin", "document"),
    ]:
        message = await send(tmp_path, name, kind=kind)
        assert of_type(message, tl.DocumentAttributeFilename).file_name == name


# --- refusals, before the bytes move ------------------------------------------


async def test_a_kind_the_file_cannot_be_is_refused(chat, tmp_path):
    """Never converted to fit the kind. Refused."""
    wire, _, _, send = chat
    with pytest.raises(ValueError) as caught:
        await send(tmp_path, "song.mp3", kind="video")
    assert "song.mp3" in str(caught.value) and "video" in str(caught.value)


async def test_the_refusal_happens_before_the_upload(chat, tmp_path):
    """The file is not encrypted, not uploaded, and no key is spent on it."""
    wire, _, _, send = chat
    before = len(wire.a.uploaded)
    with pytest.raises(ValueError):
        await send(tmp_path, "picture.jpg", kind="voice_note")
    assert len(wire.a.uploaded) == before, "the file was uploaded before being refused"


async def test_an_unknown_kind_is_refused(chat, tmp_path):
    wire, _, _, send = chat
    with pytest.raises(ValueError) as caught:
        await send(tmp_path, "thing.bin", kind="hologram")
    assert "hologram" in str(caught.value)


async def test_a_caption_on_a_captionless_kind_is_refused(chat, tmp_path):
    """Refused rather than silently dropped: a caption that vanishes is a message
    the sender believes they sent."""
    wire, _, _, send = chat
    for name, kind in [("face.webp", "sticker"), ("round.mp4", "video_note")]:
        with pytest.raises(ValueError) as caught:
            await send(tmp_path, name, kind=kind, caption="look")
        assert kind in str(caught.value)


async def test_a_caption_still_rides_on_the_kinds_that_take_one(chat, tmp_path):
    _, _, _, send = chat
    message = await send(tmp_path, "song.mp3", kind="audio", caption="the demo")
    assert message.media.caption == "the demo"
