"""Telling a voice note from a music file, the same way telegram-mcp tells them.

The two repositories named the same eight kinds and then disagreed about which one
an unnamed `.ogg` becomes: telegram-mcp read it as a voice note, this package read
it as music. Nothing notices today, because the server's secret path still runs on
TDLib; the day it does not, every untagged voice message silently becomes a track.

Measured 2026-09-20 on a real music file and one recorded the way a Telegram client
records: both Ogg, both Opus, both mono, both 48 kHz. Only the comment block
differs. The rule the owner chose: no music tag means voice note, any music tag
means audio.

These cases mirror `tests/test_ogg_voice_detection.py` in telegram-mcp deliberately.
Two implementations of one rule stay in step by being tested against the same
inputs, which is the only mechanism available while the two live in separate
repositories with no dependency between them.
"""

import pytest

from telethon_secret_chat import files, ogg_tags


def _lacing(length: int, complete: bool = True) -> list:
    """xiph framing.html: 255 means the packet continues; a value below 255 ends it,
    so a packet whose length is a multiple of 255 ends with a 0."""
    if not complete:
        assert length % 255 == 0, "a continuing part must fill whole segments"
        return [255] * (length // 255)
    return [255] * (length // 255) + [length % 255]


def _page(*parts, flags: int = 0, serial: int = 1, sequence: int = 0, version: int = 0) -> bytes:
    """One Ogg page: 27-byte header, segment table, body. ``parts`` are
    ``(bytes, complete)`` packet pieces. CRC is left zero: nothing here checks it."""
    lacing = bytes(v for data, complete in parts for v in _lacing(len(data), complete))
    assert len(lacing) <= 255
    return (
        b"OggS"
        + bytes([version, flags])
        + bytes(8)
        + serial.to_bytes(4, "little")
        + sequence.to_bytes(4, "little")
        + bytes(4)
        + bytes([len(lacing)])
        + lacing
        + b"".join(data for data, _ in parts)
    )


def _comment_packet(*comments: str, codec: bytes = b"OpusHead") -> bytes:
    """The comment header: vendor, count, then length-prefixed ``NAME=value``.

    The length prefixes matter: they are what puts a non-letter byte in front of
    every tag name, which is how `ALBUM=` is told apart from `MYALBUM=`.
    """
    magic = b"OpusTags" if codec == b"OpusHead" else b"\x03vorbis"
    block = (4).to_bytes(4, "little") + b"test" + len(comments).to_bytes(4, "little")
    for comment in comments:
        raw = comment.encode("utf-8")
        block += len(raw).to_bytes(4, "little") + raw
    return magic + block + (b"\x01" if codec != b"OpusHead" else b"")


def _ogg(*comments: str, codec: bytes = b"OpusHead", split: bool = False) -> bytes:
    """The first pages of a real Ogg stream: an identification packet alone on the
    BOS page, then the comment packet - on one page, or across two when ``split``."""
    if codec == b"OpusHead":
        ident = b"OpusHead" + b"\x01\x01" + bytes(9)
    else:
        ident = b"\x01vorbis" + bytes(23)
    first = _page((ident, True), flags=0x02)
    packet = _comment_packet(*comments, codec=codec)
    if not split:
        return first + _page((packet, True), sequence=1)
    cut = 255 * (len(packet) // 255 if len(packet) % 255 else len(packet) // 255 - 1)
    assert 0 < cut < len(packet), "split needs a comment packet longer than 255 bytes"
    return (
        first
        + _page((packet[:cut], False), sequence=1)
        + _page((packet[cut:], True), flags=0x01, sequence=2)
    )


def test_a_recording_with_no_music_tags_is_a_voice_note():
    assert ogg_tags.looks_like_voice(_ogg()) is True


def test_an_encoder_tag_alone_is_still_a_voice_note():
    assert ogg_tags.looks_like_voice(_ogg("ENCODER=Lavf62.3.100")) is True


@pytest.mark.parametrize(
    "comment", ["TITLE=A Song", "ARTIST=Someone", "ALBUM=A Record", "TRACKNUMBER=3"]
)
def test_any_music_tag_makes_it_audio(comment):
    assert ogg_tags.looks_like_voice(_ogg(comment)) is False


def test_bytes_that_are_not_ogg_say_nothing_either_way():
    assert ogg_tags.looks_like_voice(b"ID3 not an ogg") is None
    assert ogg_tags.looks_like_voice(b"") is None


def test_a_tag_name_inside_a_value_does_not_count():
    assert ogg_tags.looks_like_voice(_ogg("ENCODER=TITLEmaker 1.0")) is True


# --- and the inference that reads it -----------------------------------------


def test_an_untagged_ogg_now_infers_a_voice_note():
    """This is the behaviour change. Without a header it still infers `audio`,
    which is what this package has always done."""
    assert files._infer_kind("audio/ogg", _ogg()) == "voice_note"
    assert files._infer_kind("audio/ogg") == "audio"


def test_a_tagged_ogg_infers_audio():
    assert files._infer_kind("audio/ogg", _ogg("TITLE=A Song")) == "audio"


def test_an_explicit_kind_is_never_second_guessed():
    tagged = _ogg("TITLE=A Song")
    assert (
        files.resolve_kind(
            "voice_note", file_name="song.ogg", mime_type="audio/ogg", header=tagged
        )
        == "voice_note"
    )


def test_nothing_but_audio_is_read_at_all():
    """A jpeg is a photo by its type; there is nothing in its head worth an I/O."""
    assert files._infer_kind("image/jpeg", _ogg()) == "photo"
    assert files._infer_kind("video/mp4", _ogg()) == "video"
    assert files._infer_kind("image/gif", _ogg()) == "animation"


# --- an mp3 is not a voice note ----------------------------------------------


@pytest.mark.parametrize(
    "mime", ["audio/mpeg", "audio/mp4", "audio/flac", "audio/wav", "audio/aac"]
)
def test_no_compressed_music_format_may_be_asked_for_as_a_voice_note(mime):
    """Mirrors telegram-mcp. A voice message is Telegram's own OGG/Opus
    recording; the owner ruled on 2026-09-20 that an mp3 is not one, after the
    real-client pass showed the other side accepting a 10 MB mp3 as a voice note
    and starting the upload."""
    with pytest.raises(ValueError):
        files.resolve_kind("voice_note", file_name="track", mime_type=mime)


@pytest.mark.parametrize("mime", ["audio/ogg", "audio/opus"])
def test_telegrams_own_voice_container_may_still_be_either(mime):
    assert files.resolve_kind("voice_note", file_name="n", mime_type=mime) == "voice_note"
    assert files.resolve_kind("audio", file_name="n", mime_type=mime) == "audio"
