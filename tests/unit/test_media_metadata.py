"""Media metadata the sender supplies (spec 005, US1).

What an official client shows before download - a video's length and size, a preview, a voice
note's waveform, a sticker's emoji - comes only from these fields; the package reads nothing out
of the file. Verified on the official Android client 2026-09-29 (docs/design/media-metadata.md).
Every refusal happens before the upload: the failure that costs nothing is the early one.
"""

import pytest

from telethon_secret_chat.schema import secret_tl as tl

from .test_files_attributes import of_type

JPEG = b"\xff\xd8\xff\xe0" + b"\x00" * 60
WEBP = b"RIFF\x00\x00\x00\x00WEBPVP8 " + b"\x00" * 50


async def test_a_video_carries_its_length_size_and_preview(chat, tmp_path):
    wire, _, _, send = chat
    got = await send(
        tmp_path,
        "clip.mp4",
        duration=3,
        width=320,
        height=240,
        thumbnail=JPEG,
        thumbnail_size=(90, 68),
    )
    video = of_type(got, tl.DocumentAttributeVideo)
    assert (video.duration, video.w, video.h) == (3, 320, 240)
    assert (got.media.thumb, got.media.thumb_w, got.media.thumb_h) == (JPEG, 90, 68)


async def test_a_voice_note_carries_its_length_and_waveform(chat, tmp_path):
    _, _, _, send = chat
    got = await send(tmp_path, "note.ogg", kind="voice_note", duration=7, waveform=b"\x1f" * 63)
    audio = of_type(got, tl.DocumentAttributeAudio)
    assert audio.voice and audio.duration == 7 and audio.waveform == b"\x1f" * 63


async def test_a_song_carries_title_and_performer(chat, tmp_path):
    _, _, _, send = chat
    got = await send(tmp_path, "song.mp3", duration=200, title="T", performer="P")
    audio = of_type(got, tl.DocumentAttributeAudio)
    assert (audio.duration, audio.title, audio.performer) == (200, "T", "P")


async def test_a_sticker_carries_its_emoji_and_size(chat, tmp_path):
    _, _, _, send = chat
    got = await send(
        tmp_path,
        "s.webp",
        kind="sticker",
        sticker_alt="🙂",
        width=512,
        height=512,
        thumbnail=WEBP,
        thumbnail_size=(128, 128),
    )
    assert of_type(got, tl.DocumentAttributeSticker).alt == "🙂"
    size = of_type(got, tl.DocumentAttributeImageSize)
    assert (size.w, size.h) == (512, 512)


async def test_a_photo_carries_its_size(chat, tmp_path):
    _, _, _, send = chat
    got = await send(tmp_path, "p.jpg", width=800, height=600)
    size = of_type(got, tl.DocumentAttributeImageSize)
    assert (size.w, size.h) == (800, 600)


async def test_omitted_metadata_changes_nothing(chat, tmp_path):
    _, _, _, send = chat
    got = await send(tmp_path, "clip.mp4")
    video = of_type(got, tl.DocumentAttributeVideo)
    assert (video.duration, video.w, video.h) == (0, 0, 0)
    assert (got.media.thumb, got.media.thumb_w, got.media.thumb_h) == (b"", 0, 0)


@pytest.mark.parametrize(
    "name, kwargs",
    [
        ("p.jpg", {"waveform": b"\x01"}),  # a waveform on a photo
        ("p.jpg", {"duration": 3}),  # a length on a photo
        ("doc.pdf", {"width": 10}),  # a size on a document
        ("song.mp3", {"sticker_alt": "x"}),  # an emoji on a song
        ("clip.mp4", {"title": "T"}),  # a title on a video
        ("clip.mp4", {"duration": -1}),
        ("clip.mp4", {"width": 2**31}),
        ("clip.mp4", {"duration": True}),
        ("clip.mp4", {"thumbnail": JPEG}),  # a preview without its size
        ("clip.mp4", {"thumbnail_size": (90, 68)}),  # a size without a preview
        ("clip.mp4", {"thumbnail": JPEG, "thumbnail_size": (321, 10)}),
        ("clip.mp4", {"thumbnail": JPEG, "thumbnail_size": (0, 10)}),
        (
            "clip.mp4",
            {"thumbnail": b"\xff\xd8\xff" + b"\x00" * (200 * 1024), "thumbnail_size": (9, 9)},
        ),
        ("clip.mp4", {"thumbnail": b"not an image", "thumbnail_size": (9, 9)}),
        ("clip.mp4", {"thumbnail": WEBP, "thumbnail_size": (9, 9)}),  # WEBP is for stickers
        ("note.ogg", {"waveform": b"\x00" * 64}),
        ("song.mp3", {"title": 5}),
    ],
)
async def test_bad_metadata_is_refused_before_the_upload(chat, tmp_path, name, kwargs):
    wire, a, chat_a, _ = chat
    source = tmp_path / name
    source.write_bytes(b"x" * 64)
    if name == "note.ogg":
        kwargs = dict(kwargs, kind="voice_note")
    with pytest.raises(ValueError):
        await a.send_file(chat_a.id, source, **kwargs)
    assert wire.a.uploaded == []
