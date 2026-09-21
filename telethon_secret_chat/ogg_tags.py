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
    """Return True for voice, False for music, or None for an incomplete header.

    Parse bounded, length-prefixed comments, not tag-looking text in a vendor or
    another field's value. A comment packet spanning an unavailable page cannot
    establish that music tags are absent; uncertainty preserves the caller's
    existing fallback. No codec is decoded and no explicit kind is overridden.
    """
    if not isinstance(header, bytes) or not header.startswith(b"OggS"):
        return None
    header = header[:HEADER_BYTES]
    positions = [(header.find(magic), magic) for magic in (b"OpusTags", b"\x03vorbis")]
    positions = [(position, magic) for position, magic in positions if position >= 0]
    if not positions:
        return None
    start, magic = min(positions)
    cursor = start + len(magic)

    def uint32():
        nonlocal cursor
        if cursor + 4 > len(header):
            raise ValueError
        value = int.from_bytes(header[cursor : cursor + 4], "little")
        cursor += 4
        return value

    def field():
        nonlocal cursor
        length = uint32()
        if length > len(header) - cursor:
            raise ValueError
        value = header[cursor : cursor + length]
        cursor += length
        return value

    try:
        field()  # The vendor string is not a user comment.
        count = uint32()
        if count > (len(header) - cursor) // 4:
            return None
        music = False
        for _ in range(count):
            name, separator, _ = field().partition(b"=")
            music |= bool(separator and name.upper() in MUSIC_TAGS)
        return not music
    except ValueError:
        return None
