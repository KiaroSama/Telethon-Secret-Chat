"""Where a secret chat survives a restart, and the one rule a backend must obey.

A chat's key, its fingerprint, the second key during a rekey (§4.8) and both
sequence counters (§3.5, §3.6) are ONE fact. §1.5 derives the fingerprint from the
key, so a record holding key A with fingerprint B decrypts nothing - and from the
outside that is indistinguishable from a peer problem, which is what makes it
expensive. A backend that writes them in separate steps can be interrupted between
them and leave exactly that record.

So the interface is shaped around a single ``save`` of the whole record, and
``_commit`` is the one place a backend makes its write visible. The shared contract
suite interrupts ``_commit`` and asserts the PREVIOUS record survived intact.

There is deliberately no default backend. FR-014: a library that picks where to
write key material picks a location the operator never protected.

data-model.md §5 is the specification this implements.
"""

from __future__ import annotations

import json
import os
import tempfile
from abc import ABC, abstractmethod
from pathlib import Path
from typing import Any, Dict, List, Optional

__all__ = ["StorageBackend", "MemoryStorage", "FileStorage"]

Record = Dict[str, Any]
Message = Dict[str, Any]


class StorageBackend(ABC):
    """What an application must provide for chats to survive a restart.

    Small on purpose: the fewer operations, the fewer ways to implement it wrongly.
    Subclasses implement ``_commit`` and the queue primitives; the atomicity
    guarantee lives in ``save`` here so every backend inherits the same shape.
    """

    # --- the chat record ------------------------------------------------------

    def save(self, record: Record) -> None:
        """Persist a chat as ONE unit.

        The whole record, never a field at a time. §8 of the protocol reference
        records the archived package saving inside ``__setattr__`` - a write per
        attribute, which both multiplies writes and widens the window in which a
        partially-updated record can be observed.
        """
        self._commit(int(record["id"]), dict(record))

    @abstractmethod
    def _commit(self, chat_id: int, record: Record) -> None:
        """Make one complete record visible, atomically. The seam the contract
        suite interrupts."""

    @abstractmethod
    def load(self, chat_id: int) -> Optional[Record]:
        """The full record, or ``None``. Never a partial one."""

    @abstractmethod
    def delete(self, chat_id: int) -> None:
        """Remove the chat and both its queues."""

    @abstractmethod
    def list(self) -> List[int]:
        """Every chat id held here."""

    # --- outgoing retention (§3.7) -------------------------------------------

    @abstractmethod
    def queue_out(self, chat_id: int, message: Message) -> None:
        """Retain a sent message so a resend request can be answered."""

    @abstractmethod
    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        """Forget messages the peer has acknowledged, inclusive."""

    @abstractmethod
    def retained_out(self, chat_id: int) -> List[Message]:
        """What can still be resent, in order."""

    # --- the gap queue (§3.7) -------------------------------------------------

    @abstractmethod
    def queue_in(self, chat_id: int, message: Message) -> None:
        """Hold a message that arrived ahead of a hole.

        This queue holds PLAINTEXT, which makes it the place Principle IV is most
        easily broken. It is never logged and never included in an error.
        """

    @abstractmethod
    def take_in(self, chat_id: int) -> List[Message]:
        """Take everything held, in conversation order, and empty the queue."""


class MemoryStorage(StorageBackend):
    """For tests, and for an application that genuinely wants chats to die with the
    process. Never the default - nothing is."""

    def __init__(self) -> None:
        self._chats: Dict[int, Record] = {}
        self._out: Dict[int, List[Message]] = {}
        self._in: Dict[int, List[Message]] = {}

    def _commit(self, chat_id: int, record: Record) -> None:
        # One rebind of one name: nothing can observe a half-updated record, because
        # the dict entry either points at the old record or the new one.
        self._chats[chat_id] = record

    def load(self, chat_id: int) -> Optional[Record]:
        record = self._chats.get(chat_id)
        # A copy, so a caller mutating what it loaded cannot reach into the store.
        return dict(record) if record is not None else None

    def delete(self, chat_id: int) -> None:
        self._chats.pop(chat_id, None)
        self._out.pop(chat_id, None)
        self._in.pop(chat_id, None)

    def list(self) -> List[int]:
        return list(self._chats)

    def queue_out(self, chat_id: int, message: Message) -> None:
        self._out.setdefault(chat_id, []).append(dict(message))

    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        kept = [m for m in self._out.get(chat_id, []) if m["seq_no"] > up_to_seq]
        self._out[chat_id] = kept

    def retained_out(self, chat_id: int) -> List[Message]:
        return sorted(self._out.get(chat_id, []), key=lambda m: m["seq_no"])

    def queue_in(self, chat_id: int, message: Message) -> None:
        self._in.setdefault(chat_id, []).append(dict(message))

    def take_in(self, chat_id: int) -> List[Message]:
        held = sorted(self._in.pop(chat_id, []), key=lambda m: m["seq_no"])
        return held


class FileStorage(StorageBackend):
    """One file, one process, owner-only.

    Not safe for two processes - and that is stated rather than defended against,
    because exclusivity is a property of the Telegram session, not of this file.
    An application running two instances on one session has a larger problem than
    this backend can solve.

    Written by replace, never in place: a crash during a write leaves the previous
    file, which is the whole atomicity guarantee.
    """

    def __init__(self, path):
        self.path = Path(path)
        self._state: Dict[str, Any] = {"chats": {}, "out": {}, "in": {}}
        if self.path.exists():
            self._state = json.loads(self.path.read_text(encoding="utf-8"))
        else:
            self._write()

    # Keys are bytes; JSON has no bytes. Hex rather than base64 so a human reading
    # the file cannot mistake it for text.
    _BYTES_FIELDS = ("key", "pending_key", "previous_key")

    def _encode(self, record: Record) -> Record:
        out = dict(record)
        for field in self._BYTES_FIELDS:
            if isinstance(out.get(field), (bytes, bytearray)):
                out[field] = {"__bytes__": bytes(out[field]).hex()}
        return out

    def _decode(self, record: Record) -> Record:
        out = dict(record)
        for field in self._BYTES_FIELDS:
            value = out.get(field)
            if isinstance(value, dict) and "__bytes__" in value:
                out[field] = bytes.fromhex(value["__bytes__"])
        return out

    def _write(self) -> None:
        """Replace the file. The temp file is created beside the target so the
        replace is on one filesystem and therefore atomic."""
        directory = self.path.parent
        directory.mkdir(parents=True, exist_ok=True)
        handle, temporary = tempfile.mkstemp(dir=str(directory), suffix=".tmp")
        try:
            with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as fh:
                json.dump(self._state, fh, indent=1, sort_keys=True)
            self._restrict(Path(temporary))
            os.replace(temporary, self.path)
        except BaseException:
            Path(temporary).unlink(missing_ok=True)
            raise

    @staticmethod
    def _restrict(path: Path) -> None:
        """Owner-only, before the file carries anything.

        On POSIX the mode says it. On Windows ``chmod`` toggles the read-only
        attribute and cannot clear the read bit, so the mode is not the guarantee
        there - the file inherits the directory's ACL, and an application storing
        keys on Windows is responsible for that directory.
        """
        if os.name != "nt":
            os.chmod(path, 0o600)

    def _commit(self, chat_id: int, record: Record) -> None:
        # The whole state is rebuilt and replaced, so an interrupted write leaves
        # the previous file untouched - key, fingerprint and counters together.
        previous = self._state["chats"].get(str(chat_id))
        self._state["chats"][str(chat_id)] = self._encode(record)
        try:
            self._write()
        except BaseException:
            # In-memory state must not drift ahead of the file it failed to reach.
            if previous is None:
                self._state["chats"].pop(str(chat_id), None)
            else:
                self._state["chats"][str(chat_id)] = previous
            raise

    def load(self, chat_id: int) -> Optional[Record]:
        record = self._state["chats"].get(str(chat_id))
        return self._decode(record) if record is not None else None

    def delete(self, chat_id: int) -> None:
        self._state["chats"].pop(str(chat_id), None)
        self._state["out"].pop(str(chat_id), None)
        self._state["in"].pop(str(chat_id), None)
        self._write()

    def list(self) -> List[int]:
        return [int(k) for k in self._state["chats"]]

    def queue_out(self, chat_id: int, message: Message) -> None:
        self._state["out"].setdefault(str(chat_id), []).append(dict(message))
        self._write()

    def drop_out(self, chat_id: int, up_to_seq: int) -> None:
        held = self._state["out"].get(str(chat_id), [])
        self._state["out"][str(chat_id)] = [m for m in held if m["seq_no"] > up_to_seq]
        self._write()

    def retained_out(self, chat_id: int) -> List[Message]:
        return sorted(self._state["out"].get(str(chat_id), []), key=lambda m: m["seq_no"])

    def queue_in(self, chat_id: int, message: Message) -> None:
        self._state["in"].setdefault(str(chat_id), []).append(dict(message))
        self._write()

    def take_in(self, chat_id: int) -> List[Message]:
        held = sorted(self._state["in"].pop(str(chat_id), []), key=lambda m: m["seq_no"])
        self._write()
        return held
