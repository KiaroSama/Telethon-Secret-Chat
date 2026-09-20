"""Is this `.ogg` a voice note, or a music file?

The same question telegram-mcp answers in its own `ogg_tags.py`, answered the same
way on purpose: the two sides named the same eight kinds and then disagreed about
which one an unnamed `.ogg` becomes, which is invisible today and silently changes
every untagged voice message the day this package replaces TDLib.

The extension cannot say and neither can the codec. Measured 2026-09-20 on a real
music file and on one recorded the way a Telegram client records: both Ogg, both
Opus, both mono, both 48 kHz. What differs is the Vorbis comment block - a music
file carries `TITLE`, `ARTIST`, `ALBUM`; a recording carries at most `ENCODER`,
which every Ogg writer emits and which therefore means nothing.

The rule, decided by the owner on 2026-09-20: no music tag means voice note, any
music tag means audio.

This reads the CONTAINER, never the audio. Parsing a header is not decoding a
codec, which is the only reason it belongs in a package that holds ciphertext and
a schema and deliberately no codec: `attributes_for` still sends duration, width
and height as zero, and still will.
"""

__all__ = ["HEADER_BYTES", "MUSIC_TAGS", "looks_like_voice"]

#: How much of the file the answer needs. An Ogg page is capped at 65 307 bytes
#: and the identification header plus the comment block sit in the first pages.
HEADER_BYTES = 64 * 1024

#: The comment names that mean "a piece of music". `ENCODER` is deliberately
#: absent: every Ogg writer emits one, a phone's recorder included.
MUSIC_TAGS = (
    b"TITLE",
    b"ARTIST",
    b"ALBUM",
    b"ALBUMARTIST",
    b"TRACKNUMBER",
    b"GENRE",
    b"PERFORMER",
    b"COMPOSER",
)


def looks_like_voice(header: bytes):
    """``True`` voice note, ``False`` music, ``None`` when the bytes do not say.

    ``None`` means this reader learned nothing, not "probably music": the caller
    falls back to whatever it would have done without a reader at all.
    """
    if not header.startswith(b"OggS") or len(header) <= 4:
        return None
    if b"OpusTags" not in header and b"\x03vorbis" not in header:
        return None
    return not _has_music_tag(header)


def _has_music_tag(header: bytes) -> bool:
    """A music tag NAME, at the start of a comment, not anywhere in the bytes.

    A Vorbis comment is ``NAME=value``, so the name is what precedes the first
    `=`, and the byte before the name is framing rather than a letter. Searching
    loose for `b"TITLE"` matches an encoder whose version string contains the
    word, and demotes a real recording for no reason.
    """
    upper = header.upper()
    for tag in MUSIC_TAGS:
        start = 0
        while True:
            found = upper.find(tag + b"=", start)
            if found == -1:
                break
            if found == 0 or not upper[found - 1 : found].isalpha():
                return True
            start = found + 1
    return False
