"""Byte-capped RAM receipt; caller receives bytes, never a local content path."""

import asyncio
import io

from telethon.tl import types

from . import files
from .bounded_types import ChatPaused


class CappedBuffer(io.BytesIO):
    def __init__(self, cap, chat_id, manager):
        super().__init__()
        self.cap, self.chat_id, self.manager = cap, chat_id, manager

    def write(self, content, /):
        self.manager._admit(self.chat_id, "media-chunk")
        if self.tell() + len(content) > self.cap:
            raise ChatPaused(chat_id=self.chat_id)
        return super().write(content)

    def seek(self, offset, whence=0):
        if whence == 0 and not 0 <= offset <= self.cap:
            raise ChatPaused(chat_id=self.chat_id)
        return super().seek(offset, whence)


async def receive(manager, message, max_bytes):
    if type(max_bytes) is not int or not 0 < max_bytes <= manager.limits.bytes:
        raise ValueError("max_bytes must be positive and no greater than the custody byte budget")
    media, attached = files._file_of(message)
    manager._require(message.chat_id)
    padded = media.size + (-media.size % files.BLOCK)
    # Ciphertext, decrypted pieces and returned immutable bytes coexist briefly.
    if media.size > max_bytes or padded * 4 > manager.limits.bytes:
        raise ChatPaused(chat_id=message.chat_id)
    manager._admit(message.chat_id, "media")
    reserved = padded * 4
    if (
        manager._media_tasks + len(manager._active) >= manager.limits.tasks
        or manager._media_bytes + reserved > manager.limits.bytes
    ):
        raise ChatPaused(chat_id=message.chat_id)
    manager._media_tasks += 1
    manager._media_bytes += reserved
    try:
        async with asyncio.timeout(manager.limits.operation_seconds):
            with CappedBuffer(padded, message.chat_id, manager) as destination:
                await manager._client.download_file(
                    types.InputEncryptedFileLocation(
                        id=attached.id, access_hash=attached.access_hash
                    ),
                    destination,
                    dc_id=attached.dc_id,
                )
                manager._admit(message.chat_id, "media")
                return files.decrypt_file(destination.getvalue(), media.key, media.iv, media.size)
    except asyncio.CancelledError:
        raise
    except Exception:
        raise ChatPaused(chat_id=message.chat_id) from None
    finally:
        manager._media_tasks -= 1
        manager._media_bytes -= reserved
