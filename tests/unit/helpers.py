"""Builders shared by the unit tier. Test modules import from here, never from each other."""

from telethon.tl import functions, types

from telethon_secret_chat import SecretChatManager, crypto
from telethon_secret_chat.chat import SecretChat
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import FakeClient

# --- sequence (§3.4-§3.7) -------------------------------------------------------

SEQ_KEY = bytes((i * 5 + 3) % 256 for i in range(256))


def a_chat(sent=0):
    """We are the originator, so incoming carries (in odd, out even)."""
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.adopt_key(SEQ_KEY)
    chat.out_seq_no = sent  # how many WE have sent, for §3.6's D
    return chat


def peer_message(raw_out, text="x", raw_in=0):
    """A message from the peer, with §3.4's transform already applied."""
    return tl.DecryptedMessageLayer(
        random_bytes=b"\x00" * 31,
        layer=144,
        in_seq_no=2 * raw_in + 1,  # the peer is the recipient: x = 1
        out_seq_no=2 * raw_out,  # ... and x = 0 on its out
        message=tl.DecryptedMessage(random_id=raw_out, ttl=0, message=text),
    )


def texts(result):
    return [w.message.message for w in result.ready]


# --- a started manager holding one ready chat -------------------------------------

KEY = bytes(range(256))
OTHER = bytes(reversed(range(256)))


def ready_manager(client=None, store=None):
    manager = SecretChatManager(client or FakeClient(), storage=store or MemoryStorage())
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    chat.adopt_key(KEY)
    chat.layer = 144
    manager._chats[chat.id] = chat
    manager._save(chat)
    return manager, chat


# --- transport failures --------------------------------------------------------

SEND_TYPES = (
    functions.messages.SendEncryptedRequest,
    functions.messages.SendEncryptedServiceRequest,
    functions.messages.SendEncryptedFileRequest,
)


class FlakyClient(FakeClient):
    failing = True

    async def __call__(self, request):
        if self.failing and isinstance(request, SEND_TYPES):
            self.sent.append(request)
            raise OSError("synthetic RPC uncertainty")
        return await super().__call__(request)


def incoming(chat, wrapper, key=KEY):
    return types.EncryptedMessage(
        random_id=999,
        chat_id=chat.id,
        date=0,
        bytes=crypto.encrypt_frame(key, bytes(wrapper), chat.in_x),
        file=types.EncryptedFileEmpty(),
    )


# --- Ogg framing (xiph framing.html) ---------------------------------------------


def lacing_values(length: int, complete: bool = True) -> list:
    """xiph framing.html: 255 means the packet continues; a value below 255 ends it,
    so a packet whose length is a multiple of 255 ends with a 0."""
    if not complete:
        assert length % 255 == 0, "a continuing part must fill whole segments"
        return [255] * (length // 255)
    return [255] * (length // 255) + [length % 255]


def page(*parts, flags: int = 0, serial: int = 1, sequence: int = 0, version: int = 0) -> bytes:
    """One Ogg page: 27-byte header, segment table, body. ``parts`` are
    ``(bytes, complete)`` packet pieces. CRC is left zero: nothing here checks it."""
    lacing = bytes(v for data, complete in parts for v in lacing_values(len(data), complete))
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


def comment_packet(*comments: str, codec: bytes = b"OpusHead") -> bytes:
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


def ogg(*comments: str, codec: bytes = b"OpusHead", split: bool = False) -> bytes:
    """The first pages of a real Ogg stream: an identification packet alone on the
    BOS page, then the comment packet - on one page, or across two when ``split``."""
    if codec == b"OpusHead":
        ident = b"OpusHead" + b"\x01\x01" + bytes(9)
    else:
        ident = b"\x01vorbis" + bytes(23)
    first = page((ident, True), flags=0x02)
    packet = comment_packet(*comments, codec=codec)
    if not split:
        return first + page((packet, True), sequence=1)
    cut = 255 * (len(packet) // 255 if len(packet) % 255 else len(packet) // 255 - 1)
    assert 0 < cut < len(packet), "split needs a comment packet longer than 255 bytes"
    return (
        first
        + page((packet[:cut], False), sequence=1)
        + page((packet[cut:], True), flags=0x01, sequence=2)
    )
