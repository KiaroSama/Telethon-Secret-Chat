"""Coordinated protocol-state commits and bounded RAM-only payload queues."""

from __future__ import annotations

import time
from contextlib import contextmanager
from copy import deepcopy
from typing import Any

from .protected import PROTECTED_FIELDS, ProtectedFileStorage
from .storage import MemoryStorage
from .transient_work import TransientLimits, TransientRefused, charged


class TransientStorage(MemoryStorage):
    """Explicit split, not a backend silently dropping fields from a normal store."""

    def __init__(self, protected: ProtectedFileStorage, limits: TransientLimits) -> None:
        super().__init__()
        self.protected = protected
        self.limits = limits
        self.binding: dict[str, int] | None = None
        self.suspended: set[int] = set()
        self.created: dict[tuple, float] = {}
        self.attempts: dict[tuple, int] = {}
        self.external_bytes = 0
        self._refusing = False

    @contextmanager
    def transaction(self):
        outer = self._depth == 0
        previous = (deepcopy(self.created), deepcopy(self.attempts), set(self.suspended))
        try:
            # Memory rollback remains live until the protected outer commit ends.
            with super().transaction():
                with self.protected.transaction():
                    yield self
                    if outer and not getattr(self, "_refusing", False):
                        self.check_limits()
        except BaseException:
            self.created, self.attempts, self.suspended = previous
            raise

    def check_limits(self) -> None:
        count = sum(
            len(records) for group in ("out", "in") for records in self._state[group].values()
        )
        count += sum(len(chat["pending_deliveries"]) for chat in self._state["chats"].values())
        if (
            len(self._state["chats"]) > self.limits.chats
            or count > self.limits.records
            or charged(self._state) * 4 + self.external_bytes > self.limits.bytes
        ):
            raise TransientRefused()

    def save(self, record: dict[str, Any]) -> None:
        chat_id = record["id"]
        if chat_id in self.suspended and not self._refusing:
            raise TransientRefused(chat_id=chat_id)
        if self.binding is None:
            raise TransientRefused(chat_id=chat_id)
        with self.transaction():
            super().save(record)
            projected = {field: record[field] for field in PROTECTED_FIELDS}
            projected["closed_reason"] = None
            projected["state"] = record["state"]
            self.protected.save(
                {
                    "format": 1,
                    "binding": self.binding,
                    "chat": projected,
                    "suspended": chat_id in self.suspended,
                }
            )
            if record["pending_deliveries"]:
                self.created.setdefault(("mail", chat_id), time.monotonic())
            else:
                self.created.pop(("mail", chat_id), None)

    def queue_out(self, chat_id: int, message: dict) -> None:
        if chat_id in self.suspended:
            raise TransientRefused(chat_id=chat_id)
        with self.transaction():
            super().queue_out(chat_id, message)
            self.created.setdefault(("out", chat_id, message["seq_no"]), time.monotonic())

    def queue_in(self, chat_id: int, message: dict) -> None:
        if chat_id in self.suspended:
            raise TransientRefused(chat_id=chat_id)
        with self.transaction():
            super().queue_in(chat_id, message)
            self.created.setdefault(("in", chat_id, message["seq_no"]), time.monotonic())

    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        with self.transaction():
            super().drop_out(chat_id, up_to_seq)
            for key in list(self.created):
                if key[:2] == ("out", chat_id) and key[2] <= up_to_seq:
                    self.created.pop(key, None)
                    self.attempts.pop(key, None)

    def take_in(self, chat_id: int) -> list[dict]:
        with self.transaction():
            return super().take_in(chat_id)

    def requeue_in(self, chat_id: int, messages: list[dict]) -> None:
        with self.transaction():
            super().requeue_in(chat_id, messages)
            waiting = {item["seq_no"] for item in messages}
            for key in list(self.created):
                if key[:2] == ("in", chat_id) and key[2] not in waiting:
                    self.created.pop(key, None)

    def expired(self, chat_id: int) -> bool:
        return any(
            key[1] == chat_id and time.monotonic() - created >= self.limits.retention_seconds
            for key, created in self.created.items()
        )

    def attempt(self, chat_id: int, sequence: int) -> None:
        key = ("out", chat_id, sequence)
        if self.expired(chat_id) or self.attempts.get(key, 0) >= self.limits.attempts:
            raise TransientRefused(chat_id=chat_id)
        self.attempts[key] = self.attempts.get(key, 0) + 1

    def purge(self, chat_id: int) -> None:
        """Only AFTER suspension: RAM no longer supports protocol continuation."""
        with self.transaction():
            for group in ("out", "in"):
                self._state[group].pop(str(chat_id), None)
            if str(chat_id) in self._state["chats"]:
                self._state["chats"][str(chat_id)]["pending_deliveries"] = []
            for key in list(self.created):
                if key[1] == chat_id:
                    self.created.pop(key, None)
                    self.attempts.pop(key, None)

    def load_setting(self, name: str) -> Any:
        return None

    def save_setting(self, name: str, value: Any) -> None:
        raise TransientRefused()

    def delete(self, chat_id: int) -> None:
        raise TransientRefused(chat_id=chat_id)
