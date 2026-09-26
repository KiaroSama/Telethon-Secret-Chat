"""Comment fields, not tag-looking substrings in values, determine Ogg inference."""

import pytest

from telethon_secret_chat import ogg_tags
from .test_ogg_voice_detection import _comment_packet, _ogg, _page


@pytest.mark.parametrize(
    "comment",
    [
        "ENCODER=engine TITLE=example",
        "COMMENT= ARTIST=Nobody",
        "MYALBUM=demo",
        "ENCODER= ALBUM=not-a-tag",
    ],
)
def test_music_tag_text_inside_another_comments_value_does_not_count(comment):
    assert ogg_tags.looks_like_voice(_ogg(comment)) is True


@pytest.mark.parametrize("removed", [1, 2, 5, 10])
def test_truncated_comment_block_is_unknown_not_a_voice_verdict(removed):
    assert ogg_tags.looks_like_voice(_ogg("ENCODER=test")[:-removed]) is None


def test_lowercase_real_music_tag_still_counts():
    assert ogg_tags.looks_like_voice(_ogg("artist=somebody")) is False


# --- page-aware reassembly (specs/002-audit-gap-closure FR-002) ---------------

PAD = "ENCODER=" + "x" * 300  # pushes the comment packet past one 255-byte segment run


@pytest.mark.parametrize("codec", [b"OpusHead", b"\x01vorbis"])
def test_a_music_tag_in_a_comment_packet_split_across_pages_counts(codec):
    assert ogg_tags.looks_like_voice(_ogg(PAD, "TITLE=A Song", codec=codec, split=True)) is False


@pytest.mark.parametrize("codec", [b"OpusHead", b"\x01vorbis"])
def test_an_untagged_comment_packet_split_across_pages_is_a_voice_note(codec):
    assert ogg_tags.looks_like_voice(_ogg(PAD, codec=codec, split=True)) is True


def test_magic_bytes_outside_the_comment_packet_are_not_a_header():
    ident = b"OpusHead" + b"\x01\x01" + bytes(9)
    stream = (
        _page((ident, True), flags=0x02)
        + _page((b"audio frame", True), sequence=1)
        + _page((_comment_packet("TITLE=A Song"), True), sequence=2)
    )
    assert ogg_tags.looks_like_voice(stream) is None


def test_a_page_of_another_logical_stream_is_skipped():
    ident = b"OpusHead" + b"\x01\x01" + bytes(9)
    stream = (
        _page((ident, True), flags=0x02, serial=1)
        + _page((_comment_packet("TITLE=Other stream"), True), serial=2)
        + _page((_comment_packet(), True), serial=1, sequence=1)
    )
    assert ogg_tags.looks_like_voice(stream) is True


def _broken():
    ident = b"OpusHead" + b"\x01\x01" + bytes(9)
    good = _ogg("ENCODER=x")
    return {
        "bad capture pattern": b"OggX" + good[4:],
        "unknown stream structure version": good[:4] + b"\x01" + good[5:],
        "first page is not beginning of stream": good[:5] + b"\x00" + good[6:],
        "lacing runs past the buffer": good[:-3],
        "packet unfinished at buffer end": _page((ident, True), flags=0x02)
        + _page((_comment_packet(PAD)[:255], False), sequence=1),
        "continuation flag with nothing to continue": _page((ident, True), flags=0x03)
        + _page((_comment_packet(), True), sequence=1),
        "first packet is not an identification header": _page((b"garbage", True), flags=0x02)
        + _page((_comment_packet(), True), sequence=1),
    }


@pytest.mark.parametrize("case", list(_broken()))
def test_malformed_framing_is_unknown(case):
    assert ogg_tags.looks_like_voice(_broken()[case]) is None
