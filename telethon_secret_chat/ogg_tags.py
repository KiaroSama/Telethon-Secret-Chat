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
    if not isinstance(header, bytes):
        return None
    packets = _first_packets(header[:HEADER_BYTES], 2)
    if packets is None:
        return None
    ident, comment = packets
    # RFC 7845 §5 and Vorbis I §4.2: packet 1 identifies the codec, packet 2 is
    # the comment header. Magic anywhere else is audio data, not a header.
    for codec, magic in ((b"OpusHead", b"OpusTags"), (b"\x01vorbis", b"\x03vorbis")):
        if ident.startswith(codec):
            break
    else:
        return None
    if not comment.startswith(magic):
        return None
    return _no_music_tags(comment[len(magic) :])


def _first_packets(data: bytes, wanted: int):
    """The first ``wanted`` packets of the first logical stream, or None.

    xiph.org/ogg/doc/framing.html: a 27-byte page header (``OggS``, version 0,
    flags, granule, serial, sequence, CRC, segment count), the lacing table, then
    the body. A lacing value of 255 continues the packet, anything below ends it,
    and a packet may run on into the next page, which then sets flag 0x01. The CRC
    is not checked: this classifies, it does not validate.
    """
    packets, current, serial, position = [], b"", None, 0
    while len(packets) < wanted:
        page = data[position : position + 27]
        if len(page) < 27 or page[:4] != b"OggS" or page[4] != 0:
            return None
        flags, count = page[5], page[26]
        lacing = data[position + 27 : position + 27 + count]
        body = position + 27 + count
        end = body + sum(lacing)
        if len(lacing) < count or end > len(data):
            return None
        if serial is None:
            if not flags & 0x02:
                return None  # the first page of a stream is its beginning
            serial = page[14:18]
        if page[14:18] == serial:
            if bool(flags & 0x01) != bool(current):
                return None  # a continuation with nothing open, or an open packet dropped
            for size in lacing:
                current += data[body : body + size]
                body += size
                if size < 255:
                    packets.append(current)
                    current = b""
                    if len(packets) == wanted:
                        break
        position = end
    return packets


def _no_music_tags(block: bytes):
    """Read the vendor string and the length-prefixed ``NAME=value`` comments."""
    cursor = 0

    def uint32():
        nonlocal cursor
        if cursor + 4 > len(block):
            raise ValueError
        value = int.from_bytes(block[cursor : cursor + 4], "little")
        cursor += 4
        return value

    def field():
        nonlocal cursor
        length = uint32()
        if length > len(block) - cursor:
            raise ValueError
        value = block[cursor : cursor + length]
        cursor += length
        return value

    try:
        field()  # The vendor string is not a user comment.
        count = uint32()
        if count > (len(block) - cursor) // 4:
            return None
        music = False
        for _ in range(count):
            name, separator, _ = field().partition(b"=")
            music |= bool(separator and name.upper() in MUSIC_TAGS)
        return not music
    except ValueError:
        return None
