"""Public finite custody configuration and content-free pause diagnostics."""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any

from .errors import SecretChatError


@dataclass(frozen=True)
class CustodyLimits:
    """Per-chat retained records/serialized bytes, plus global chat/work ceilings.

    Required limits are application policy, not a promise of permanent retention.
    ``bytes`` measures serialized custody, not process RSS or external client queues.
    """

    records: int
    bytes: int
    age_seconds: float
    chats: int = 64
    frame_bytes: int = 256 * 1024
    operation_seconds: float = 15
    tasks: int = 8

    def __post_init__(self):
        for name, ceiling in (
            ("records", 4096),
            ("bytes", 64 * 1024 * 1024),
            ("chats", 64),
            ("frame_bytes", 1024 * 1024),
            ("tasks", 64),
        ):
            value = getattr(self, name)
            if type(value) is not int or not 0 < value <= ceiling:
                raise ValueError(f"{name} must be a positive integer no greater than {ceiling}")
        for name, ceiling in (("age_seconds", 30 * 86400), ("operation_seconds", 60)):
            value = getattr(self, name)
            if (
                type(value) not in (int, float)
                or not math.isfinite(value)
                or not 0 < value <= ceiling
            ):
                raise ValueError(f"{name} must be positive and finite, no greater than {ceiling}")


class ChatPaused(SecretChatError):
    """No new effect admitted; previously committed custody is preserved."""

    def __init__(self, *, chat_id=None):
        self.reason = "bounded conversation paused; inspect pause_status before explicit resume"
        super().__init__(self.reason, chat_id=chat_id)


@dataclass(frozen=True)
class PauseStatus:
    chat_id: int
    paused: bool
    resumable: bool
    reason: str | None
    records: int
    bytes: int


@dataclass(frozen=True)
class RelayDelivery:
    """Stable incoming sequence identity; ``message`` contains sensitive material."""

    chat_id: int
    seq_no: int
    message: Any = field(repr=False)
