"""Finite budgets and content-free failures for opt-in transient handling."""

from __future__ import annotations

import asyncio
import math
from dataclasses import dataclass, fields
from functools import wraps
from typing import Any

from .errors import SecretChatError


class TransientRefused(SecretChatError):
    """Transient processing cannot continue safely; no payload is included."""

    reason = "transient processing refused"

    def __init__(self, *, chat_id: int | None = None) -> None:
        super().__init__(self.reason, chat_id=chat_id)


class TransientCleanupIncomplete(TransientRefused):
    """Owned work resisted cancellation; shutdown has NOT completed."""

    reason = "transient cleanup incomplete; owned work remains"


@dataclass(frozen=True)
class TransientLimits:
    """Validated finite budgets; bytes charge serialized content, not process RSS."""

    chats: int = 64
    records: int = 512
    bytes: int = 8 * 1024 * 1024
    frame_bytes: int = 256 * 1024
    retention_seconds: float = 300.0
    operation_seconds: float = 15.0
    cleanup_seconds: float = 5.0
    tasks: int = 8
    attempts: int = 3

    def __post_init__(self) -> None:
        ceilings = {
            "chats": 64,
            "records": 4096,
            "bytes": 64 * 1024 * 1024,
            "frame_bytes": 1024 * 1024,
            "retention_seconds": 3600,
            "operation_seconds": 60,
            "cleanup_seconds": 30,
            "tasks": 64,
            "attempts": 10,
        }
        for item in fields(self):
            value = getattr(self, item.name)
            timed = item.name.endswith("seconds")
            valid_type = type(value) in (int, float) if timed else type(value) is int
            if not valid_type or not math.isfinite(value) or not 0 < value <= ceilings[item.name]:
                raise ValueError("transient limits must be positive, finite and within ceilings")
        if self.frame_bytes > self.bytes:
            raise ValueError("the frame budget must fit inside the memory budget")


def charged(value: Any) -> int:
    """Conservative content accounting with container overhead; no object repr."""
    if value is None:
        return 8
    if isinstance(value, str):
        return 64 + len(value.encode("utf-8")) * 4
    if isinstance(value, (bytes, bytearray)):
        return 64 + len(value)
    if type(value) in (int, float, bool):
        return 64
    if isinstance(value, dict):
        return 128 + sum(charged(k) + charged(v) for k, v in value.items())
    if isinstance(value, (list, tuple)):
        return 64 + sum(charged(v) for v in value)
    # Protocol objects are serialized only in RAM; never repr or pickle them.
    if hasattr(value, "__bytes__"):
        return 128 + len(bytes(value)) * 4
    from .media import MediaReference

    if isinstance(value, MediaReference):
        return charged(value.to_dict())
    raise TransientRefused()


def bounded(method):
    """Public inherited operation, sharing one owned slot across reentrant calls."""

    @wraps(method)
    async def run(self, *args, **kwargs):
        chat_id = args[0] if args and type(args[0]) is int else kwargs.get("chat_id")
        try:
            size = charged(args) + charged(kwargs)
        except Exception:
            raise TransientRefused(chat_id=chat_id) from None
        return await self._execute(method, args, kwargs, chat_id, size)

    return run


async def cancel_owned(tasks: set, seconds: float) -> None:
    """Never hide a cancellation-resistant task behind a successful shutdown."""
    pending = {task for task in tasks if task is not asyncio.current_task() and not task.done()}
    for task in pending:
        if not task.cancelling():
            task.cancel()
    if pending:
        _, unfinished = await asyncio.wait(pending, timeout=seconds)
        if unfinished:
            raise TransientCleanupIncomplete()
