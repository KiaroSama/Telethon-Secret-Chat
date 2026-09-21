"""Atomic persistence for a chat, its outgoing retention, and its incoming gaps.

A counter committed without its message is just as corrupt as a key committed
without its fingerprint. ``transaction`` therefore covers ALL storage operations,
not only ``save``. It is synchronous: never hold a transaction across an await.
Custom backends must implement this contract; silently emulating it with several
independent writes cannot provide crash consistency.

FileStorage is one-process storage, not an inter-process/session lock. On Windows
its directory must have an owner-only ACL configured by the application.
"""

from __future__ import annotations

import json
import logging
import os
import tempfile
import threading
from abc import ABC, abstractmethod
from contextlib import contextmanager
from copy import deepcopy
from pathlib import Path
from typing import Any, Dict, List, Optional

__all__ = ["StorageBackend", "MemoryStorage", "FileStorage"]

Record = Dict[str, Any]
Message = Dict[str, Any]
log = logging.getLogger("telethon_secret_chat.storage")


class StorageBackend(ABC):
    """Storage operations return detached copies, never mutable internal state."""

    def save(self, record: Record) -> None:
        self._commit(int(record["id"]), deepcopy(record))

    @abstractmethod
    def transaction(self):
        """Synchronous context manager: all changes commit together or roll back.

        Nested scopes must be supported. A failed outer commit restores both the
        durable state and the backend's in-memory view. No network or await may
        occur inside the scope. Custom backends should use their native database
        transaction rather than attempt compensating writes after a failure.
        """

    @abstractmethod
    def _commit(self, chat_id: int, record: Record) -> None:
        """Make one complete record visible, within the current transaction."""

    @abstractmethod
    def load(self, chat_id: int) -> Optional[Record]:
        """The full detached record, or None."""

    @abstractmethod
    def delete(self, chat_id: int) -> None:
        """Remove the chat and both queues atomically."""

    @abstractmethod
    def list(self) -> List[int]:
        """Every chat ID held here."""

    @abstractmethod
    def queue_out(self, chat_id: int, message: Message) -> None:
        """Upsert one retained outgoing record by its assigned sequence number."""

    @abstractmethod
    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        """Forget acknowledged outgoing records, inclusive."""

    @abstractmethod
    def retained_out(self, chat_id: int) -> List[Message]:
        """Detached retained outgoing records in sequence order."""

    @abstractmethod
    def queue_in(self, chat_id: int, message: Message) -> None:
        """Retain the first copy of a gap message; duplicates must not grow storage."""

    def peek_in(self, chat_id: int) -> List[Message]:
        """Inspect gaps without consuming them; native backends should optimize this."""
        with self.transaction():
            items = self.take_in(chat_id)
            for item in items:
                self.queue_in(chat_id, item)
        return items

    @abstractmethod
    def take_in(self, chat_id: int) -> List[Message]:
        """Drain the gap queue in order, within the current transaction."""


class MemoryStorage(StorageBackend):
    """Explicit volatile storage with the same transaction contract as FileStorage."""

    def __init__(self) -> None:
        self._state: Dict[str, Any] = {"chats": {}, "out": {}, "in": {}}
        self._lock = threading.RLock()
        self._depth = 0

    @contextmanager
    def transaction(self):
        with self._lock:
            previous = deepcopy(self._state)
            self._depth += 1
            try:
                yield self
                if self._depth == 1 and self._state != previous:
                    self._write()
            except BaseException:
                self._state = previous
                raise
            finally:
                self._depth -= 1

    def _write(self) -> None:
        """Volatile storage has no durable commit step."""

    def _commit(self, chat_id: int, record: Record) -> None:
        with self.transaction():
            self._state["chats"][str(chat_id)] = deepcopy(record)

    def load(self, chat_id: int) -> Optional[Record]:
        with self._lock:
            return deepcopy(self._state["chats"].get(str(chat_id)))

    def delete(self, chat_id: int) -> None:
        with self.transaction():
            for group in ("chats", "out", "in"):
                self._state[group].pop(str(chat_id), None)

    def list(self) -> List[int]:
        with self._lock:
            return [int(k) for k in self._state["chats"]]

    def queue_out(self, chat_id: int, message: Message) -> None:
        with self.transaction():
            held = self._state["out"].setdefault(str(chat_id), [])
            for index, item in enumerate(held):
                if item["seq_no"] == message["seq_no"]:
                    held[index] = deepcopy(message)
                    break
            else:
                held.append(deepcopy(message))

    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        with self.transaction():
            held = self._state["out"].get(str(chat_id), [])
            self._state["out"][str(chat_id)] = [m for m in held if m["seq_no"] > up_to_seq]

    def retained_out(self, chat_id: int) -> List[Message]:
        with self._lock:
            held = self._state["out"].get(str(chat_id), [])
            return deepcopy(sorted(held, key=lambda m: m["seq_no"]))

    def queue_in(self, chat_id: int, message: Message) -> None:
        with self.transaction():
            held = self._state["in"].setdefault(str(chat_id), [])
            if not any(m["seq_no"] == message["seq_no"] for m in held):
                held.append(deepcopy(message))

    def peek_in(self, chat_id: int) -> List[Message]:
        with self._lock:
            held = self._state["in"].get(str(chat_id), [])
            return deepcopy(sorted(held, key=lambda m: m["seq_no"]))

    def take_in(self, chat_id: int) -> List[Message]:
        with self.transaction():
            held = self._state["in"].pop(str(chat_id), [])
            return deepcopy(sorted(held, key=lambda m: m["seq_no"]))


class FileStorage(MemoryStorage):
    """Whole-state replace, with rollback for EVERY mutation and durable file data.

    Existing JSON stores remain readable. Keys are tagged byte strings; the store
    is not encrypted at rest. Protect the containing directory and backups.
    """

    def __init__(self, path):
        super().__init__()
        self.path = Path(path)
        if self.path.is_symlink():
            raise ValueError("the secret-chat store must not be a symbolic link")
        if self.path.exists():
            try:
                state = json.loads(self.path.read_text(encoding="utf-8"), object_hook=self._decode)
                if not isinstance(state, dict) or any(
                    not isinstance(state.get(group), dict) for group in ("chats", "out", "in")
                ):
                    raise ValueError
                self._state = state
            except (ValueError, TypeError, UnicodeError):
                raise ValueError("the secret-chat store is not a valid storage document") from None
            self._restrict(self.path)
        else:
            self._write()

    @staticmethod
    def _encode(value):
        if isinstance(value, (bytes, bytearray)):
            return {"__bytes__": bytes(value).hex()}
        raise TypeError("the storage record contains an unsupported value type")

    @staticmethod
    def _decode(value):
        if set(value) == {"__bytes__"}:
            return bytes.fromhex(value["__bytes__"])
        return value

    def _write(self) -> None:
        directory = self.path.parent
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        handle, temporary = tempfile.mkstemp(dir=str(directory), suffix=".tmp")
        try:
            # mkstemp is owner-only on POSIX. Apply the restriction before data.
            self._restrict(Path(temporary))
            with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as fh:
                handle = None
                json.dump(self._state, fh, default=self._encode, indent=1, sort_keys=True)
                fh.flush()
                os.fsync(fh.fileno())
            os.replace(temporary, self.path)
        except BaseException:
            if handle is not None:
                os.close(handle)
            Path(temporary).unlink(missing_ok=True)
            raise
        # A directory fsync failure occurs AFTER replace has committed. It cannot
        # be reported as a rollback: that would put memory behind the durable file.
        if os.name != "nt":
            descriptor = None
            try:
                descriptor = os.open(directory, os.O_RDONLY)
                os.fsync(descriptor)
            except OSError:
                log.warning("storage committed, but directory durability could not be confirmed")
            finally:
                if descriptor is not None:
                    os.close(descriptor)

    @staticmethod
    def _restrict(path: Path) -> None:
        if os.name != "nt":
            os.chmod(path, 0o600)
