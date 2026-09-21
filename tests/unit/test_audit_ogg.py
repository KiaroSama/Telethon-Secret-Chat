"""Comment fields, not tag-looking substrings in values, determine Ogg inference."""

import pytest

from telethon_secret_chat import ogg_tags
from .test_ogg_voice_detection import _ogg


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
