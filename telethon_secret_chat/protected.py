"""Strict protected-state file boundary. No mailbox or wire frame is accepted."""

from __future__ import annotations

import json
from copy import deepcopy
from pathlib import Path
from typing import Any

from .chat import SecretChat
from .storage import FileStorage
from .transient_work import TransientRefused

PROTECTED_FIELDS = frozenset(
    "id access_hash peer_user_id is_outbound key key_fingerprint pending_key previous_key "
    "exchange_id exchange_secret rekey_role dh_prime dh_g in_seq_no out_seq_no peer_in_seq_no "
    "gap_requested resend_due layer wrapper_layer ttl created_at rekeyed_at messages_since_rekey "
    "admin_id participant_id closed_reason handshake gap_end new_key_confirmed initial_key_hash".split()
)
ENVELOPE_FIELDS = {"format", "binding", "chat", "suspended"}
MAX_RECORD = 64 * 1024
MAX_STORE = 4 * 1024 * 1024
MAX_CHATS = 64


def validate_binding(binding: Any) -> None:
    if not isinstance(binding, dict) or set(binding) != {"user_id", "auth_key_id", "dc_id"}:
        raise TransientRefused()
    if not all(type(value) is int for value in binding.values()):
        raise TransientRefused()
    if not (
        0 < binding["user_id"] < 2**63
        and -(2**63) <= binding["auth_key_id"] < 2**64
        and binding["auth_key_id"] != 0
        and 0 < binding["dc_id"] <= 1000
    ):
        raise TransientRefused()


def validate_envelope(record: Any, chat_id: int | None = None) -> None:
    if not isinstance(record, dict) or set(record) != ENVELOPE_FIELDS:
        raise TransientRefused(chat_id=chat_id)
    if type(record["format"]) is not int or record["format"] != 1:
        raise TransientRefused(chat_id=chat_id)
    validate_binding(record["binding"])
    if type(record["suspended"]) is not bool:
        raise TransientRefused(chat_id=chat_id)
    chat = record["chat"]
    if not isinstance(chat, dict) or set(chat) != PROTECTED_FIELDS | {"state"}:
        raise TransientRefused(chat_id=chat_id)
    handshake = chat["handshake"]
    if not isinstance(handshake, dict) or not set(handshake) <= {"p", "g", "g_a", "secret"}:
        raise TransientRefused(chat_id=chat_id)
    if chat["closed_reason"] is not None:
        raise TransientRefused(chat_id=chat_id)
    if any(value is not None and type(value) is not int for value in handshake.values()):
        raise TransientRefused(chat_id=chat_id)
    # Existing validation permits missing legacy values. This boundary admits only
    # exact typed fields, never a string in a key/counter/DH slot carrying a payload.
    try:
        SecretChat.from_record(dict(chat, pending_deliveries=[]), stored_id=chat_id)
        encoded = json.dumps(record, default=FileStorage._encode, ensure_ascii=True)
    except Exception:
        raise TransientRefused(chat_id=chat_id) from None
    if len(encoded.encode("utf-8")) > MAX_RECORD:
        raise TransientRefused(chat_id=chat_id)


class ProtectedFileStorage(FileStorage):
    """Atomic protected envelopes only; legacy documents are never migrated.

    The containing Windows directory must already have an owner-only ACL. This is
    one-process storage, not encryption at rest or an inter-process lock.
    """

    def __init__(self, path: str | Path) -> None:
        try:
            given = Path(path)
            if given.exists() and given.stat().st_size > MAX_STORE:
                raise TransientRefused()
            super().__init__(given)
        except Exception:
            raise TransientRefused() from None
        if set(self._state) != {"chats", "out", "in", "mode"}:
            # A fresh FileStorage has no mode; only an empty newly created document
            # is eligible, never an existing ordinary store even when empty.
            raise TransientRefused()
        self._validate_state()

    def _validate_state(self) -> None:
        if set(self._state) != {"chats", "out", "in", "mode"}:
            raise TransientRefused()
        if self._state.get("mode") != "protected-v1" or self._state["out"] or self._state["in"]:
            raise TransientRefused()
        if len(self._state["chats"]) > MAX_CHATS:
            raise TransientRefused()
        for chat_id, record in self._state["chats"].items():
            try:
                stored_id = int(chat_id)
            except (ValueError, TypeError):
                raise TransientRefused() from None
            validate_envelope(record, stored_id)

    def _write(self) -> None:
        # FileStorage calls _write during new-file construction; marking an empty
        # fresh store here does not convert any preexisting ordinary state.
        if not self.path.exists() and self._state == {"chats": {}, "out": {}, "in": {}}:
            self._state["mode"] = "protected-v1"
        self._validate_state()
        if len(json.dumps(self._state, default=self._encode).encode("utf-8")) > MAX_STORE:
            raise TransientRefused()
        super()._write()

    def save(self, record: dict[str, Any]) -> None:
        validate_envelope(record)
        chat_id = record["chat"]["id"]
        with self.transaction():
            self._state["chats"][str(chat_id)] = deepcopy(record)

    def queue_out(self, chat_id: int, message: dict) -> None:
        raise TransientRefused(chat_id=chat_id)

    def queue_in(self, chat_id: int, message: dict) -> None:
        raise TransientRefused(chat_id=chat_id)

    def save_setting(self, name: str, value: Any) -> None:
        raise TransientRefused()

    def delete(self, chat_id: int) -> None:
        raise TransientRefused(chat_id=chat_id)

    def _commit(self, chat_id: int, record: dict) -> None:
        raise TransientRefused(chat_id=chat_id)
