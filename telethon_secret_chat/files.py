"""Encrypted files and their one-time keys - protocol-reference.md §6.

A file in a secret chat is encrypted TWICE over, by two unrelated schemes, and
keeping them apart is most of this module.

- The file's bytes get a **one-time** key and IV (§6.1), "in no way related to the
  chat's shared key", AES-IGE with no ``x`` and no ``msg_key``. The server stores
  only ciphertext.
- The key and IV then travel INSIDE an ordinary encrypted message (§6.3), while the
  file's address travels outside it in cleartext to the server.

§6.2's fingerprint is the join between the two, and it is a **different algorithm
from §1.5's**: ``md5(key + iv)`` folded to 32 bits, not the last 64 bits of a SHA-1.
Reusing §1.5's routine here produces a plausible number that no official client
agrees with, so the two live in different modules with different names.

§6 is the healthiest area of the archived package - §8.6 found the keys, the
fingerprint and the fold all correct there. What it lacked was forwarding and
big-file awareness; the ordering guarantee below (FR-012: nothing written before the
fingerprint matches) is stated here rather than assumed.
"""

from __future__ import annotations

import hashlib
import io
import mimetypes
import os
import secrets
import tempfile
from pathlib import Path

from telethon.crypto import AES
from telethon.tl import types

from . import framing, ogg_tags
from .errors import MessageRejected
from .schema import secret_tl as tl

__all__ = [
    "send",
    "receive",
    "new_file_key",
    "file_fingerprint",
    "verify_file_fingerprint",
    "encrypt_file",
    "decrypt_file",
    "save",
    "MEDIA_KINDS",
    "CAPTIONLESS_KINDS",
    "resolve_kind",
    "attributes_for",
]

BLOCK = 16
# protocol-reference.md §7.1: layer 143 made `size` a long (big files).
SIZE_LONG_LAYER = 143
TEMP_PREFIX = ".secret-chat-file-"

# --- the eight kinds (§6.3's `attributes:Vector<DocumentAttribute>`) -----------
# The vector is the only thing that tells a receiving client an `.ogg` is a voice
# note rather than a file to download, and a filename alone says "file" about all
# eight. The names are the ones the MCP tools already speak, so a consumer moving
# off TDLib keeps the same vocabulary rather than translating between two.

MEDIA_KINDS = (
    "photo",
    "video",
    "document",
    "audio",
    "animation",
    "sticker",
    "video_note",
    "voice_note",
)

#: The two with nowhere to put a caption. `decryptedMessageMediaDocument` has the
#: field, but no client shows one on a sticker or a round video, so a caption
#: offered with either is refused rather than sent somewhere it will not appear.
CAPTIONLESS_KINDS = frozenset({"sticker", "video_note"})

# What a file must already BE for each kind, matched on the mime family. `document`
# is absent because it takes anything. Nothing here converts a file to fit a kind:
# a mismatch is refused, since the alternative is transcoding media inside a
# library whose one job is that the bytes reach the peer unaltered.
_KIND_NEEDS = {
    "photo": ("image/",),
    "sticker": ("image/",),
    "animation": ("image/gif", "video/"),
    "video": ("video/",),
    "video_note": ("video/",),
    "audio": ("audio/",),
    # A voice message is Telegram's own OGG/Opus recording, not any audio file.
    # telegram-mcp learned this the hard way on 2026-09-20: its family table let
    # a 10 MB mp3 be asked for as a voice note and the upload started. The two
    # sides agree name for name, so they agree here too.
    "voice_note": ("audio/ogg", "audio/opus", "audio/x-opus"),
}


def _peek(source: Path) -> bytes:
    """The head of the file, or nothing at all.

    Read errors are swallowed on purpose: the kind refusal must stay the FIRST
    thing that can fail, exactly as it is without this, so an unreadable file
    still reports being unreadable at the read below rather than here. An empty
    result simply means the caller decides on the name, as it always did.
    """
    try:
        with source.open("rb") as handle:
            return handle.read(ogg_tags.HEADER_BYTES)
    except OSError:
        return b""


def guess_mime(file_name: str) -> str:
    """The file's type from its name, or the type that means "unknown"."""
    return mimetypes.guess_type(file_name)[0] or "application/octet-stream"


def _infer_kind(mime_type: str, header: bytes = b"") -> str:
    """What an unasked-for file is.

    `sticker` and `video_note` are never inferred: a `.webp` is a sticker only if
    the sender meant one, and guessing turns an ordinary send into a kind the
    sender did not choose.

    `voice_note` used to be in that list, and this is where the two repositories
    silently disagreed - telegram-mcp read an unnamed `.ogg` as a voice note and
    this package read it as music. An `.ogg` holds both in the same container,
    codec, channel count and sample rate, so neither default is right more than
    half the time. Given the file's head, the comment block decides instead; given
    nothing, the old answer stands. See `ogg_tags` and telegram-mcp's
    `docs/adr/0004-an-ogg-is-a-voice-note-until-its-tags-say-otherwise.md`.
    """
    if mime_type == "image/gif":
        return "animation"
    for family, kind in (("image/", "photo"), ("video/", "video"), ("audio/", "audio")):
        if mime_type.startswith(family):
            if kind == "audio" and header:
                voice = ogg_tags.looks_like_voice(header)
                if voice is not None:
                    return "voice_note" if voice else "audio"
            return kind
    return "document"


def resolve_kind(
    kind, *, file_name: str, mime_type: str, caption: str = "", header: bytes = b""
) -> str:
    """Settle the kind, or refuse - and refuse before the caller spends an upload.

    ``None`` infers. A named kind is checked against what the file is, and an
    unknown mime type is not proof of anything, so it passes: the check exists to
    catch a file that demonstrably cannot be what was asked for, not to require a
    recognised extension.
    """
    if kind is None:
        kind = _infer_kind(mime_type, header)
    elif kind not in MEDIA_KINDS:
        raise ValueError(f"unknown media kind {kind!r}: expected one of {', '.join(MEDIA_KINDS)}")
    needs = _KIND_NEEDS.get(kind, ())
    if needs and mime_type != "application/octet-stream":
        if not any(
            mime_type.startswith(family) if family.endswith("/") else mime_type == family
            for family in needs
        ):
            raise ValueError(
                f"{file_name} cannot be sent as a {kind}: it is {mime_type}. "
                "Send it as a document, or convert it first - this package will not."
            )
    if caption and kind in CAPTIONLESS_KINDS:
        raise ValueError(
            f"a {kind} carries no caption, and {file_name} was given one. "
            "The protocol has no field a client would show it in; send the text "
            "as its own message."
        )
    return kind


def attributes_for(kind: str, file_name: str) -> list:
    """The kind's own attributes, always alongside the filename.

    The numbers - duration, width, height - are left at zero: reading them means
    decoding the media, and this package holds ciphertext and a schema, not a codec.
    A client shows the kind from the attribute's presence and its flags; the
    dimensions refine the preview it draws.
    """
    attributes = [tl.DocumentAttributeFilename(file_name=file_name)]
    if kind == "photo":
        attributes.append(tl.DocumentAttributeImageSize(w=0, h=0))
    elif kind in ("video", "video_note"):
        attributes.append(
            tl.DocumentAttributeVideo(round_message=kind == "video_note", duration=0, w=0, h=0)
        )
    elif kind in ("audio", "voice_note"):
        attributes.append(tl.DocumentAttributeAudio(voice=kind == "voice_note", duration=0))
    elif kind == "animation":
        attributes.append(tl.DocumentAttributeAnimated())
    elif kind == "sticker":
        attributes.append(
            tl.DocumentAttributeSticker(alt="", stickerset=tl.InputStickerSetEmpty())
        )
    return attributes


def new_file_key() -> tuple[bytes, bytes]:
    """§6.1: "2 random 256-bit numbers ... which will serve as the AES key and
    initialization vector used to encrypt the file".

    Fresh per file, from the OS CSPRNG, and taking no argument at all - the surest
    way to keep "in no way related to the chat's shared key" true is to give the
    function no way to see one.
    """
    return os.urandom(32), os.urandom(32)


def file_fingerprint(key: bytes, iv: bytes) -> int:
    """§6.2: ``digest = md5(key + iv)``; ``substr(digest, 0, 4) XOR substr(digest, 4, 4)``.

    MD5 and 32 bits, against §1.5's SHA-1 and 64. It rides outside the encrypted
    message, in the ``encryptedFile`` the server returns, so a receiver can check
    that the key it found INSIDE the decrypted message belongs to the file attached
    OUTSIDE it. Read as a little-endian signed int32, the width of
    ``encryptedFile.key_fingerprint``.
    """
    digest = hashlib.md5(key + iv).digest()
    folded = bytes(a ^ b for a, b in zip(digest[0:4], digest[4:8]))
    return int.from_bytes(folded, "little", signed=True)


def verify_file_fingerprint(*, chat_id: int, key: bytes, iv: bytes, claimed: int) -> None:
    """§6.3: "a mismatch is a rejection, not a warning".

    Neither the key nor the IV appears in the error. They are one-time, but they are
    still the material that opens this file (Principle IV).
    """
    if file_fingerprint(key, iv) != claimed:
        raise MessageRejected(
            chat_id=chat_id,
            reason=(
                "the file's key fingerprint does not match the key carried in the "
                "message: the attached file does not belong to this message"
            ),
        )


# Streamed in pieces of this size, a multiple of the cipher block. Telethon's own
# `upload_file(key=, iv=)` cannot be used: it restarts the IV on every part, which is
# not an IGE stream (telethon/client/uploads.py, 1.45.0).
CHUNK = 64 * 1024


def _check_file_key(key: bytes, iv: bytes) -> None:
    if len(key) != 32 or len(iv) != 32:
        raise ValueError("file key and IV must each contain exactly 32 bytes")


class _Ige:
    """AES-256-IGE carried across pieces: after each piece the IV becomes the last
    ciphertext block followed by the last plaintext block, exactly as one call over
    the whole buffer would have continued."""

    def __init__(self, key: bytes, iv: bytes):
        self._key, self._iv = key, iv

    def encrypt(self, plain: bytes) -> bytes:
        cipher = AES.encrypt_ige(plain, self._key, self._iv)
        self._iv = cipher[-BLOCK:] + plain[-BLOCK:]
        return cipher

    def decrypt(self, cipher: bytes) -> bytes:
        plain = AES.decrypt_ige(cipher, self._key, self._iv)
        self._iv = cipher[-BLOCK:] + plain[-BLOCK:]
        return plain


class EncryptingReader:
    """A readable stream of a file's ciphertext, for `upload_file`, without holding
    the file in memory. ``size`` is the padded length the upload must declare."""

    def __init__(self, source, key: bytes, iv: bytes, size: int, *, padding=None, name=None):
        _check_file_key(key, iv)
        self._source, self._left = source, size
        self._pad = os.urandom(-size % BLOCK) if padding is None else padding
        self._ige = _Ige(key, iv)
        self._plain = b""
        self._ready = b""
        self.size = size + len(self._pad)
        self.name = name

    def read(self, count: int = -1) -> bytes:
        if count is None or count < 0:
            count = self.size
        while len(self._ready) < count and (self._left or self._pad is not None):
            piece = self._source.read(min(CHUNK, self._left)) if self._left else b""
            if self._left and not piece:
                raise ValueError("the file ended before its declared size")
            self._left -= len(piece)
            if not self._left:
                piece += self._pad
                self._pad = None
            self._plain += piece
            whole = len(self._plain) - len(self._plain) % BLOCK
            if whole:
                self._ready += self._ige.encrypt(self._plain[:whole])
                self._plain = self._plain[whole:]
        out, self._ready = self._ready[:count], self._ready[count:]
        return out


def decrypt_stream(pieces, key: bytes, iv: bytes, size: int):
    """Plaintext pieces of a ciphertext stream, trimmed to the declared ``size``."""
    _check_file_key(key, iv)
    ige, pending, left = _Ige(key, iv), b"", size
    for piece in pieces:
        pending += piece
        whole = len(pending) - len(pending) % BLOCK
        if whole and left:
            plain = ige.decrypt(pending[:whole])
            yield plain[:left]
            left -= min(left, len(plain))
        pending = pending[whole:]


def _check_lengths(total: int, size) -> None:
    if type(size) is not int or size < 0:
        raise ValueError("file size must be a nonnegative integer")
    if total % BLOCK or not 0 <= total - size < BLOCK:
        raise ValueError("ciphertext length does not match the declared file size")


def encrypt_file(content: bytes, key: bytes, iv: bytes) -> bytes:
    """§6.1: AES-256-IGE "in like manner" to §2.4, but with this file's own key.

    Padded to a cipher block because IGE has no partial block. The true length
    travels inside the message (``size`` in the media constructor), which is how the
    padding comes off again.
    """
    return EncryptingReader(io.BytesIO(content), key, iv, len(content)).read()


def decrypt_file(ciphertext: bytes, key: bytes, iv: bytes, size: int) -> bytes:
    """The reverse, trimmed to the length the message declared."""
    _check_file_key(key, iv)
    _check_lengths(len(ciphertext), size)
    return b"".join(decrypt_stream([ciphertext], key, iv, size))


def save(
    path,
    *,
    ciphertext,
    key: bytes,
    iv: bytes,
    size: int,
    claimed_fingerprint: int,
    chat_id: int,
) -> Path:
    """Write a received file, or refuse before anything reaches the disk.

    FR-012 and US5 scenario 3: the fingerprint check runs FIRST, so a file whose key
    does not belong to it is "refused rather than written to disk" - not written and
    then deleted, which leaves the bytes in a directory and in a filesystem journal
    for however long the failure takes to notice.
    """
    verify_file_fingerprint(chat_id=chat_id, key=key, iv=iv, claimed=claimed_fingerprint)
    _check_file_key(key, iv)
    # `ciphertext` is bytes or a readable binary file (a download on disk).
    stream = io.BytesIO(ciphertext) if isinstance(ciphertext, (bytes, bytearray)) else ciphertext
    start = stream.tell()
    total = stream.seek(0, os.SEEK_END) - start
    stream.seek(start)
    _check_lengths(total, size)
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # A kill mid-write leaves decrypted plaintext in a temp file; ours carry a prefix
    # so the next save can remove them.
    for leftover in target.parent.glob(TEMP_PREFIX + "*.tmp"):
        leftover.unlink(missing_ok=True)
    descriptor, temporary = tempfile.mkstemp(
        dir=str(target.parent), prefix=TEMP_PREFIX, suffix=".tmp"
    )
    try:
        with os.fdopen(descriptor, "wb") as handle:
            descriptor = None
            pieces = iter(lambda: stream.read(CHUNK), b"")
            for plain in decrypt_stream(pieces, key, iv, size):
                handle.write(plain)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, target)
    except BaseException:
        if descriptor is not None:
            os.close(descriptor)
        Path(temporary).unlink(missing_ok=True)
        raise
    return target


# --- the two operations contracts/public-api.md §2 names ----------------------
# They take the manager rather than living on it: §6 is one section and belongs in
# one module, and the manager stays the orchestrator rather than the place protocol
# rules accumulate.


async def send(
    manager, chat, path, caption: str = "", mime_type=None, kind=None, reply_to=None
) -> int:
    """§6.3-§6.4: encrypt with a one-time key, upload the ciphertext, send the
    address outside the message and the key inside it.

    §6.4: "the bytes are IGE-encrypted client-side BEFORE upload.saveFilePart, so
    the server stores ciphertext only".
    """
    source = Path(path)
    mime_type = mime_type or guess_mime(source.name)
    # Before the read, the key and the upload: a kind the file cannot be costs
    # nothing to refuse here and an encrypted round trip to refuse later.
    kind = resolve_kind(
        kind,
        file_name=source.name,
        mime_type=mime_type,
        caption=caption,
        header=_peek(source),
    )
    # An unreadable file refuses here, before a key is generated - contracts §2.
    with source.open("rb") as handle:
        size = os.fstat(handle.fileno()).st_size
        if framing.outgoing_layer(chat.layer) < SIZE_LONG_LAYER and size >= 2**31:
            raise ValueError("the negotiated layer cannot encode this file size")
        key, iv = new_file_key()
        reader = EncryptingReader(handle, key, iv, size, name=source.name)
        uploaded = await manager._client.upload_file(
            reader, file_size=reader.size, file_name=source.name
        )
    # The upload ran without the chat lock, so the chat may have moved on meanwhile.
    async with manager._chat_lock(chat.id):
        return await _send_uploaded(
            manager, chat, source, uploaded, size, key, iv, caption, mime_type, kind, reply_to
        )


async def _send_uploaded(
    manager, chat, source, uploaded, size, key, iv, caption, mime_type, kind, reply_to
) -> int:
    chat.require_sendable()
    await manager._rekey_if_due(chat)
    random_id = secrets.randbits(63)
    media_type = (
        tl.DecryptedMessageMediaDocument
        if framing.outgoing_layer(chat.layer) >= SIZE_LONG_LAYER
        else tl.DecryptedMessageMediaDocument_7afe8ae2
    )
    message = tl.DecryptedMessage(
        random_id=random_id,
        ttl=chat.ttl,
        message=caption,
        reply_to_random_id=reply_to,
        media=media_type(
            thumb=b"",
            thumb_w=0,
            thumb_h=0,
            mime_type=mime_type,
            # Use size:long only when the peer supports the layer-143 shape.
            size=size,
            key=key,
            iv=iv,
            attributes=attributes_for(kind, source.name),
            caption=caption,
        ),
    )
    await manager._send(
        chat,
        message,
        file=(
            types.InputEncryptedFileBigUploaded(
                id=uploaded.id,
                parts=uploaded.parts,
                key_fingerprint=file_fingerprint(key, iv),
            )
            if isinstance(uploaded, types.InputFileBig)
            else types.InputEncryptedFileUploaded(
                id=uploaded.id,
                parts=uploaded.parts,
                md5_checksum=uploaded.md5_checksum,
                key_fingerprint=file_fingerprint(key, iv),
            )
        ),
    )
    return random_id


async def receive(manager, message, path) -> Path:
    """§6.3: the key comes from INSIDE the decrypted message, the fingerprint from
    OUTSIDE it. Comparing them is what says the two belong together - and it happens
    before a byte is written (FR-012)."""
    media, attached = message.media, message.file
    if (
        media is None
        or attached is None
        or not all(hasattr(media, name) for name in ("key", "iv", "size"))
        or not isinstance(attached, types.EncryptedFile)
    ):
        raise MessageRejected(chat_id=message.chat_id, reason="this message carries no file")
    verify_file_fingerprint(
        chat_id=message.chat_id, key=media.key, iv=media.iv, claimed=attached.key_fingerprint
    )
    if (
        len(media.key) != 32
        or len(media.iv) != 32
        or type(media.size) is not int
        or media.size < 0
    ):
        raise MessageRejected(chat_id=message.chat_id, reason="invalid encrypted-file metadata")
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # The ciphertext goes to disk, not memory; it is useless without the key.
    with tempfile.TemporaryFile(dir=str(target.parent)) as download:
        await manager._client.download_file(
            types.InputEncryptedFileLocation(id=attached.id, access_hash=attached.access_hash),
            download,
            dc_id=attached.dc_id,
        )
        download.seek(0)
        return save(
            target,
            ciphertext=download,
            key=media.key,
            iv=media.iv,
            size=media.size,
            claimed_fingerprint=attached.key_fingerprint,
            chat_id=message.chat_id,
        )
