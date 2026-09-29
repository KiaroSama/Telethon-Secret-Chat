"""What a file carries besides its bytes, and where the bytes come from (spec 005).

Three things live here: the media metadata a sender supplies (checked before anything is
uploaded), the sources ``send_file`` reads (a path, bytes, or a seekable stream), and
``MediaReference``, which keeps a received file reachable after a restart.
"""

from __future__ import annotations

import contextlib
import io
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from telethon.extensions import BinaryReader
from telethon.tl import types

from . import ogg_tags
from .schema import secret_tl as tl

__all__ = ["MediaReference", "check_metadata", "media_of", "Source", "METADATA_FIELDS"]

# TDLib ``inputThumbnail`` (td_api.tl, tdlib/td 42e6a52, line 5801): JPEG, WEBP for stickers,
# "less than 200 KB", sides that "usually shouldn't exceed 320". Adopted as hard limits.
THUMB_MAX_BYTES = 200 * 1024
THUMB_MAX_SIDE = 320
# TDLib's voice-note waveform is 5-bit samples (td_api.tl line 620); 100 of them are 63 bytes.
WAVEFORM_MAX_BYTES = 63
_INT32_MAX = 2**31 - 1

# Which kinds each field applies to; a field absent here applies to every kind.
_APPLIES = {
    "duration": {"video", "video_note", "audio", "voice_note"},
    "width": {"photo", "video", "video_note", "animation", "sticker"},
    "height": {"photo", "video", "video_note", "animation", "sticker"},
    "waveform": {"voice_note"},
    "title": {"audio"},
    "performer": {"audio"},
    "sticker_alt": {"sticker"},
}
METADATA_FIELDS = tuple(_APPLIES) + ("thumbnail", "thumbnail_size")


def _is_jpeg(data) -> bool:
    return data[:3] == b"\xff\xd8\xff"


def _is_webp(data) -> bool:
    return data[:4] == b"RIFF" and data[8:12] == b"WEBP"


def check_metadata(kind: str, metadata: dict) -> dict:
    """The metadata, refused field by field before any upload; ``None`` values are dropped.

    Raises: ValueError naming the field and the rule, never the value.
    """
    given = {name: value for name, value in metadata.items() if value is not None}
    for name in given:
        kinds = _APPLIES.get(name)
        if kinds is not None and kind not in kinds:
            raise ValueError(f"{name} does not apply to a {kind}")
    for name in ("duration", "width", "height"):
        value = given.get(name)
        if value is not None and (type(value) is not int or not 0 <= value <= _INT32_MAX):
            raise ValueError(f"{name} must be an integer from 0 to 2**31-1")
    for name in ("title", "performer", "sticker_alt"):
        if name in given and not isinstance(given[name], str):
            raise ValueError(f"{name} must be text")
    if "waveform" in given:
        waveform = given["waveform"]
        if (
            not isinstance(waveform, (bytes, bytearray))
            or not 0 < len(waveform) <= WAVEFORM_MAX_BYTES
        ):
            raise ValueError(f"waveform must be 1 to {WAVEFORM_MAX_BYTES} bytes (5-bit samples)")
        given["waveform"] = bytes(waveform)
    _check_thumbnail(kind, given)
    return given


def _check_thumbnail(kind: str, given: dict) -> None:
    thumb, size = given.get("thumbnail"), given.get("thumbnail_size")
    if (thumb is None) != (size is None):
        raise ValueError("thumbnail and thumbnail_size go together")
    if thumb is None:
        return
    if not isinstance(thumb, (bytes, bytearray)) or not 0 < len(thumb) < THUMB_MAX_BYTES:
        raise ValueError("thumbnail must be bytes under 200 KB")
    if not (_is_jpeg(thumb) or (kind == "sticker" and _is_webp(thumb))):
        raise ValueError("thumbnail must be a JPEG (or a WEBP for a sticker)")
    if (
        not isinstance(size, tuple)
        or len(size) != 2
        or not all(type(side) is int and 1 <= side <= THUMB_MAX_SIDE for side in size)
    ):
        raise ValueError("thumbnail_size must be (width, height), each 1 to 320")
    given["thumbnail"] = bytes(thumb)


class Source:
    """A file to send: a path, bytes, or a seekable binary stream.

    No temporary file is written. A stream's size is what remains from its position, found by
    seeking, because the upload declares the size before the first byte.
    """

    def __init__(self, source, file_name: Optional[str]):
        self._path = self._data = self._stream = None
        if isinstance(source, (str, os.PathLike)):
            self._path = Path(source)
            self.name = file_name or self._path.name
            return
        if not file_name:
            raise ValueError("file_name is required when the file is given as bytes or a stream")
        self.name = file_name
        if isinstance(source, (bytes, bytearray, memoryview)):
            self._data = bytes(source)
        elif hasattr(source, "read") and hasattr(source, "seekable"):
            if not source.seekable():
                raise ValueError("a stream must be seekable, so its size is known before upload")
            self._stream = source
        else:
            raise ValueError("a file must be a path, bytes, or a seekable binary stream")

    def peek(self) -> bytes:
        """The head, for the kind check. A path's read errors are left to ``open``."""
        if self._data is not None:
            return self._data[: ogg_tags.HEADER_BYTES]
        if self._stream is not None:
            start = self._stream.tell()
            head = self._stream.read(ogg_tags.HEADER_BYTES)
            self._stream.seek(start)
            return head or b""
        assert self._path is not None
        try:
            with self._path.open("rb") as handle:
                return handle.read(ogg_tags.HEADER_BYTES)
        except OSError:
            return b""

    @contextlib.contextmanager
    def open(self):
        """Yields ``(readable, size)``."""
        if self._data is not None:
            yield io.BytesIO(self._data), len(self._data)
        elif self._stream is not None:
            start = self._stream.tell()
            end = self._stream.seek(0, io.SEEK_END)
            self._stream.seek(start)
            yield self._stream, end - start
        else:
            assert self._path is not None
            with self._path.open("rb") as handle:
                yield handle, os.fstat(handle.fileno()).st_size


def media_of(message):
    media, attached = getattr(message, "media", None), getattr(message, "file", None)
    if (
        media is None
        or not isinstance(attached, types.EncryptedFile)
        or not all(hasattr(media, name) for name in ("key", "iv", "size"))
    ):
        return None, None
    return media, attached


@dataclass(frozen=True, repr=False)
class MediaReference:
    """Everything needed to save or forward one received file later.

    It holds the file's one-time key: whoever keeps a reference can read that file, so where
    it is stored is the application's decision (docs/design/cutover-history.md, option A). The
    key and iv never appear in its repr or in an error it raises (Principle IV).

    ``media`` is the received ``DecryptedMessageMedia`` exactly as it arrived, serialized: the
    key, iv, size, mime type, preview, attributes and whether it is a photo or a document.
    """

    chat_id: int
    file_id: int
    access_hash: int
    dc_id: int
    key_fingerprint: int
    media: bytes

    FORMAT = 1

    @classmethod
    def from_message(cls, message) -> Optional["MediaReference"]:
        """The reference of a received message, or ``None`` when it carries no file."""
        media, attached = media_of(message)
        if media is None:
            return None
        return cls(
            chat_id=message.chat_id,
            file_id=attached.id,
            access_hash=attached.access_hash,
            dc_id=attached.dc_id,
            key_fingerprint=attached.key_fingerprint,
            media=bytes(media),
        )

    def decoded(self):
        """The media object, rebuilt from its bytes."""
        with BinaryReader(self.media) as reader:
            return tl.read_object(reader)

    def encrypted_file(self) -> types.EncryptedFile:
        return types.EncryptedFile(
            id=self.file_id,
            access_hash=self.access_hash,
            size=self.size,
            dc_id=self.dc_id,
            key_fingerprint=self.key_fingerprint,
        )

    @property
    def size(self) -> int:
        return self.decoded().size

    @property
    def mime_type(self) -> Optional[str]:
        media = self.decoded()
        if isinstance(media, tl.DecryptedMessageMediaPhoto):
            return "image/jpeg"
        return getattr(media, "mime_type", None)

    def to_dict(self) -> dict:
        """JSON-safe plain data, the key included (hex inside ``media``)."""
        return {
            "version": self.FORMAT,
            "chat_id": self.chat_id,
            "file_id": self.file_id,
            "access_hash": self.access_hash,
            "dc_id": self.dc_id,
            "key_fingerprint": self.key_fingerprint,
            "media": self.media.hex(),
        }

    @classmethod
    def from_dict(cls, data: dict) -> "MediaReference":
        """Raises: ValueError naming the first bad field, never its value."""
        if not isinstance(data, dict) or data.get("version") != cls.FORMAT:
            raise ValueError("unsupported media reference version")
        for name in ("chat_id", "file_id", "access_hash", "dc_id", "key_fingerprint"):
            if type(data.get(name)) is not int:
                raise ValueError(f"media reference field {name} must be an integer")
        try:
            media = bytes.fromhex(data["media"])
            with BinaryReader(media) as reader:
                decoded = tl.read_object(reader)
        except Exception:
            raise ValueError("media reference field media is not a serialized media") from None
        if not (
            isinstance(getattr(decoded, "key", None), bytes)
            and len(decoded.key) == 32
            and isinstance(getattr(decoded, "iv", None), bytes)
            and len(decoded.iv) == 32
            and type(getattr(decoded, "size", None)) is int
        ):
            raise ValueError("media reference field media carries no usable file key")
        return cls(
            chat_id=data["chat_id"],
            file_id=data["file_id"],
            access_hash=data["access_hash"],
            dc_id=data["dc_id"],
            key_fingerprint=data["key_fingerprint"],
            media=media,
        )

    def __repr__(self) -> str:
        return f"MediaReference(chat_id={self.chat_id}, file_id={self.file_id})"

    __str__ = __repr__
