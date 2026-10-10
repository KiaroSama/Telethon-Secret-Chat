"""Memory-only media receipt; no ciphertext or plaintext temporary file."""

from __future__ import annotations

import io
from typing import Any

from telethon.tl import types

from . import files
from .transient_work import TransientRefused


class BoundedDownload(io.BytesIO):
    """A hard writer ceiling, checked before each allocation."""

    def __init__(self, ceiling: int) -> None:
        super().__init__()
        self.ceiling = ceiling

    def write(self, content: Any, /) -> int:
        if self.tell() + len(content) > self.ceiling:
            raise TransientRefused()
        return super().write(content)

    def seek(self, offset: int, whence: int = 0) -> int:
        position = super().seek(offset, whence)
        if not 0 <= position <= self.ceiling:
            raise TransientRefused()
        return position


async def receive_file(manager: Any, message: Any) -> bytes:
    """Receive into finite RAM, verifying existing file fingerprint/length rules."""
    media, attached = files._file_of(message)
    manager._require(message.chat_id)
    padded = media.size + (-media.size % 16)
    # Both ciphertext and plaintext coexist during existing IGE decryption.
    if padded * 4 > manager.limits.bytes:
        raise TransientRefused(chat_id=message.chat_id)
    with BoundedDownload(padded) as destination:
        await manager._client.download_file(
            types.InputEncryptedFileLocation(id=attached.id, access_hash=attached.access_hash),
            destination,
            dc_id=attached.dc_id,
        )
        manager._require(message.chat_id)
        return files.decrypt_file(destination.getvalue(), media.key, media.iv, media.size)
