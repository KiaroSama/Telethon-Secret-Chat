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
from pathlib import Path

from telethon.crypto import AES
from telethon.tl import types

from . import ogg_tags
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
    "voice_note": ("audio/",),
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
        if not any(mime_type.startswith(family) for family in needs):
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


def encrypt_file(content: bytes, key: bytes, iv: bytes) -> bytes:
    """§6.1: AES-256-IGE "in like manner" to §2.4, but with this file's own key.

    Padded to a cipher block because IGE has no partial block. The true length
    travels inside the message (``size`` in the media constructor), which is how the
    padding comes off again.
    """
    padding = -len(content) % BLOCK
    return AES.encrypt_ige(content + os.urandom(padding), key, iv)


def decrypt_file(ciphertext: bytes, key: bytes, iv: bytes, size: int) -> bytes:
    """The reverse, trimmed to the length the message declared."""
    return AES.decrypt_ige(ciphertext, key, iv)[:size]


def save(
    path,
    *,
    ciphertext: bytes,
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
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(decrypt_file(ciphertext, key, iv, size))
    return target


# --- the two operations contracts/public-api.md §2 names ----------------------
# They take the manager rather than living on it: §6 is one section and belongs in
# one module, and the manager stays the orchestrator rather than the place protocol
# rules accumulate.


async def send(manager, chat, path, caption: str = "", mime_type=None, kind=None) -> int:
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
    content = source.read_bytes()
    key, iv = new_file_key()

    uploaded = await manager._client.upload_file(
        io.BytesIO(encrypt_file(content, key, iv)), file_name=source.name
    )
    random_id = secrets.randbits(63)
    message = tl.DecryptedMessage(
        random_id=random_id,
        ttl=chat.ttl,
        message=caption,
        media=tl.DecryptedMessageMediaDocument(
            thumb=b"",
            thumb_w=0,
            thumb_h=0,
            mime_type=mime_type,
            # `size:long`: the layer-143 shape (§6.3, §7.1). The `size:int`
            # predecessor belongs to layers this package does not announce.
            size=len(content),
            key=key,
            iv=iv,
            attributes=attributes_for(kind, source.name),
            caption=caption,
        ),
    )
    await manager._send(
        chat,
        message,
        file=types.InputEncryptedFileUploaded(
            id=uploaded.id,
            parts=uploaded.parts,
            md5_checksum="",
            key_fingerprint=file_fingerprint(key, iv),
        ),
    )
    return random_id


async def receive(manager, message, path) -> Path:
    """§6.3: the key comes from INSIDE the decrypted message, the fingerprint from
    OUTSIDE it. Comparing them is what says the two belong together - and it happens
    before a byte is written (FR-012)."""
    media, attached = message.media, message.file
    if media is None or attached is None:
        raise MessageRejected(chat_id=message.chat_id, reason="this message carries no file")
    buffer = io.BytesIO()
    await manager._client.download_file(
        types.InputEncryptedFileLocation(id=attached.id, access_hash=attached.access_hash),
        buffer,
    )
    return save(
        path,
        ciphertext=buffer.getvalue(),
        key=media.key,
        iv=media.iv,
        size=media.size,
        claimed_fingerprint=attached.key_fingerprint,
        chat_id=message.chat_id,
    )
