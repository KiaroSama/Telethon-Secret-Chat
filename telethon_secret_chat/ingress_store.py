"""Authenticated encrypted whole-state custody with exclusive process ownership."""

from __future__ import annotations

import json
import os
from pathlib import Path
import secrets
import stat
import tempfile

from .errors import SecretChatError
from .storage import FileStorage, MemoryStorage

MAGIC = b"TSC-INGRESS-1\x00"


class IngressRefused(SecretChatError):
    """Strong custody is unavailable; no sensitive exception or payload is exposed."""

    reason = "strong ingress custody refused"

    def __init__(self):
        super().__init__(self.reason)


class EncryptedIngressStorage(MemoryStorage):
    """Caller-key AEAD custody, not an archive or protection from old snapshot replay.

    Lock ownership lasts until explicit close. All state and settings share one
    authenticated atomic replacement. Directory fsync is required on POSIX; on
    Windows FlushFileBuffers via fsync protects file data, with owner ACL supplied
    by the caller. No filesystem can guarantee durability after hardware failure.
    """

    def __init__(self, path, *, key, binding, max_bytes):
        super().__init__()
        self._closed = True
        self._owner = None
        from .ingress_types import IngressLimits

        self._ingress_limits: IngressLimits | None = None
        if type(key) is not bytes or len(key) != 32:
            raise IngressRefused()
        if type(max_bytes) is not int or not 4096 <= max_bytes <= 64 * 1024 * 1024:
            raise IngressRefused()
        if not (
            isinstance(binding, dict)
            and set(binding) == {"user_id", "auth_key_id", "dc_id", "generation"}
            and all(type(binding[name]) is int for name in ("user_id", "auth_key_id", "dc_id"))
            and binding["user_id"] > 0
            and binding["dc_id"] > 0
            and isinstance(binding["generation"], str)
            and 1 <= len(binding["generation"]) <= 128
        ):
            raise IngressRefused()
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM

        self.path = Path(path).absolute()
        if any(parent.is_symlink() for parent in (self.path, *self.path.parents)):
            raise IngressRefused()
        self.max_bytes = max_bytes
        self._binding = json.loads(json.dumps(binding))
        self._aad = MAGIC + json.dumps(binding, sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
        self._cipher = AESGCM(key)
        self._pid = os.getpid()
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            self._acquire()
            self._closed = False
            if self.path.exists():
                if self.path.stat().st_size > max_bytes + 64:
                    raise IngressRefused()
                data = self.path.read_bytes()
                if not data.startswith(MAGIC):
                    raise IngressRefused()
                start = len(MAGIC)
                plaintext = self._cipher.decrypt(
                    data[start : start + 12], data[start + 12 :], self._aad
                )
                state = json.loads(plaintext.decode("utf-8"), object_hook=FileStorage._decode)
                if (
                    not isinstance(state, dict)
                    or any(
                        not isinstance(state.get(group), dict) for group in ("chats", "out", "in")
                    )
                    or not isinstance(state.get("settings", {}), dict)
                ):
                    raise IngressRefused()
                self._state = state
            else:
                self._write()
        except Exception:
            self.close()
            raise IngressRefused() from None

    @property
    def binding(self):
        return dict(self._binding)

    def _acquire(self):
        path = self.path.with_name(self.path.name + ".owner")
        if path.is_symlink():
            raise IngressRefused()
        descriptor = os.open(path, os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
        self._owner = os.fdopen(descriptor, "r+b", buffering=0)
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise IngressRefused()
        if os.name == "nt":
            import msvcrt

            if os.fstat(descriptor).st_size == 0:
                self._owner.write(b"\x00")
            self._owner.seek(0)
            msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
        else:
            import importlib

            fcntl = importlib.import_module("fcntl")
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def _check_owner(self):
        if self._closed or self._owner is None or os.getpid() != self._pid:
            raise IngressRefused()

    def _write(self):
        self._check_owner()
        self._validate_aggregate()
        plaintext = json.dumps(
            self._state,
            default=FileStorage._encode,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
        ).encode("utf-8")
        if len(plaintext) > self.max_bytes:
            raise IngressRefused()
        nonce = secrets.token_bytes(12)
        sealed = MAGIC + nonce + self._cipher.encrypt(nonce, plaintext, self._aad)
        descriptor, name = tempfile.mkstemp(prefix=".ingress-", dir=self.path.parent)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(sealed)
                stream.flush()
                os.fsync(stream.fileno())
            if self.path.is_symlink():
                raise IngressRefused()
            os.replace(name, self.path)
            if os.name != "nt":
                directory = os.open(self.path.parent, os.O_RDONLY)
                try:
                    os.fsync(directory)
                finally:
                    os.close(directory)
        except Exception:
            # Never allow an acknowledgment on an uncertain durability outcome.
            # Reopen under the owner lock to recover what actually reached disk.
            self._closed = True
            raise IngressRefused() from None
        finally:
            Path(name).unlink(missing_ok=True)

    def _validate_aggregate(self):
        """Protocol queues, batch bodies and metadata share the same byte/count cap."""
        limits = getattr(self, "_ingress_limits", None)
        if limits is None:
            return
        from .bounded_storage import size_of

        state = self._state
        checkpoint = state.get("settings", {}).get("durable_ingress_v1", {})
        controls = state.get("settings", {}).get("bounded_secret_chat_custody_v1", {})
        count = len(state["chats"]) + len(controls) + len(checkpoint.get("batches", []))
        count += sum(len(values) for group in ("out", "in") for values in state[group].values())
        count += sum(len(chat.get("pending_deliveries", [])) for chat in state["chats"].values())
        count += sum(len(control.get("relay", [])) for control in controls.values())
        if count > limits.records or size_of(state) + 256 > limits.bytes:
            raise IngressRefused()

    def transaction(self):
        self._check_owner()
        return super().transaction()

    def load(self, chat_id):
        self._check_owner()
        return super().load(chat_id)

    def list(self):
        self._check_owner()
        return super().list()

    def load_setting(self, name):
        self._check_owner()
        return super().load_setting(name)

    def retained_out(self, chat_id):
        self._check_owner()
        return super().retained_out(chat_id)

    def peek_in(self, chat_id):
        self._check_owner()
        return super().peek_in(chat_id)

    def close(self):
        self._closed = True
        if self._owner is not None:
            self._owner.close()
            self._owner = None
