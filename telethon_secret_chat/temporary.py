"""Temporary-file ownership: a filename prefix is not proof that a writer died.

Writers register before exposing their names to another thread and keep ownership
until replacement or cleanup finishes. Names carry the process id, so another
process's live or unqueryable writer is never removed. PID reuse only postpones
cleanup. Legacy names have no owner evidence and require offline operator cleanup.
"""

from __future__ import annotations

import logging
import os
import re
import sys
import tempfile
import threading
from pathlib import Path

log = logging.getLogger("telethon_secret_chat")
_lock = threading.RLock()
_active: set[Path] = set()


def _alive(pid: int) -> bool:
    """Unknown ownership is live for cleanup purposes; never terminate a process."""
    if pid == os.getpid():
        return True
    if sys.platform == "win32":
        # os.kill(pid, 0) is NOT a liveness probe on Windows. Use a query handle.
        import ctypes
        from ctypes import wintypes

        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel.OpenProcess.restype = wintypes.HANDLE
        kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        kernel.WaitForSingleObject.restype = wintypes.DWORD
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        kernel.CloseHandle.restype = wintypes.BOOL
        handle = kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE, not terminate
        if not handle:
            return ctypes.get_last_error() != 87  # ERROR_INVALID_PARAMETER: no such PID
        try:
            return kernel.WaitForSingleObject(handle, 0) != 0  # only WAIT_OBJECT_0 is dead
        finally:
            kernel.CloseHandle(handle)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except (OSError, OverflowError, ValueError):
        return True
    return True


def cleanup(directory: Path, prefix: str) -> None:
    """Remove only inactive owned leftovers; preserve live, unknown and legacy ones."""
    pattern = re.compile(re.escape(prefix) + r"([1-9][0-9]*)-[a-z0-9_]+\.tmp\Z")
    with _lock:
        for path in directory.glob(prefix + "*.tmp"):
            match = pattern.fullmatch(path.name)
            if not match or path.is_symlink():
                # A legacy writer may still have this open. Ownership cannot be guessed.
                log.debug("temporary file retained: ownership could not be established")
                continue
            pid = int(match[1])
            if pid > 0xFFFFFFFF or path.resolve() in _active:
                continue
            if pid != os.getpid() and _alive(pid):
                continue
            try:
                path.unlink(missing_ok=True)
            except OSError as failure:
                log.warning("temporary cleanup deferred: %s", type(failure).__name__)


def create(directory: Path, prefix: str) -> tuple[int, str]:
    """Create and register a restricted file; the caller owns and closes its descriptor."""
    with _lock:
        cleanup(directory, prefix)
        descriptor, name = tempfile.mkstemp(
            dir=str(directory), prefix=f"{prefix}{os.getpid()}-", suffix=".tmp"
        )
        _active.add(Path(name).resolve())
        return descriptor, name


def release(name: str) -> None:
    """Drop ownership only after the writer has closed and replaced/removed its file."""
    with _lock:
        _active.discard(Path(name).resolve())
