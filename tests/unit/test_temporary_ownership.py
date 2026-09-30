"""Cleanup requires evidence of death, not a shared filename prefix."""

import os
import subprocess
import sys
from pathlib import Path

import pytest

from telethon_secret_chat import temporary

PREFIX = ".secret-chat-file-"


def test_live_other_process_is_preserved_and_exited_process_is_reaped(tmp_path):
    # stdin EOF releases the child; no sleep or timing-based death inference.
    child = subprocess.Popen(
        [sys.executable, "-c", "import sys; sys.stdin.buffer.read()"], stdin=subprocess.PIPE
    )
    leftover = tmp_path / f"{PREFIX}{child.pid}-owned.tmp"
    leftover.write_bytes(b"synthetic plaintext")
    try:
        assert temporary._alive(child.pid)
        temporary.cleanup(tmp_path, PREFIX)
        assert leftover.exists()
    finally:
        child.communicate(timeout=5)
    assert not temporary._alive(child.pid)
    temporary.cleanup(tmp_path, PREFIX)
    assert not leftover.exists()


def test_active_same_process_is_registered_until_release(tmp_path):
    descriptor, name = temporary.create(tmp_path, PREFIX)
    try:
        temporary.cleanup(tmp_path, PREFIX)
        assert Path(name).exists()
    finally:
        os.close(descriptor)
        temporary.release(name)
    temporary.cleanup(tmp_path, PREFIX)
    assert not Path(name).exists()


def test_legacy_unknown_and_unrelated_files_are_not_deleted(tmp_path):
    names = [f"{PREFIX}legacy.tmp", f"{PREFIX}4294967296-x.tmp", ".unrelated.tmp"]
    for name in names:
        (tmp_path / name).write_bytes(b"untouched")
    temporary.cleanup(tmp_path, PREFIX)
    assert all((tmp_path / name).read_bytes() == b"untouched" for name in names)


def test_unqueryable_owner_is_preserved(tmp_path, monkeypatch):
    other = os.getpid() + 100000
    path = tmp_path / f"{PREFIX}{other}-owned.tmp"
    path.write_bytes(b"untouched")
    monkeypatch.setattr(temporary, "_alive", lambda pid: True)
    temporary.cleanup(tmp_path, PREFIX)
    assert path.exists()


def test_cleanup_failure_is_redacted_and_deferred(tmp_path, monkeypatch, caplog):
    path = tmp_path / f"{PREFIX}{os.getpid()}-inactive.tmp"
    path.write_bytes(b"synthetic")

    def denied(*args, **kwargs):
        raise PermissionError("secret exception payload")

    monkeypatch.setattr(Path, "unlink", denied)
    temporary.cleanup(tmp_path, PREFIX)
    assert path.exists()
    assert "PermissionError" in caplog.text
    assert "secret exception payload" not in caplog.text


def test_pid_outside_signed_native_range_is_conservatively_preserved(monkeypatch):
    if sys.platform == "win32":
        assert not temporary._alive(0)  # ERROR_INVALID_PARAMETER; no user process owns PID 0.
        return

    def not_representable(pid, signal):
        raise OverflowError("synthetic native pid boundary")

    monkeypatch.setattr(os, "kill", not_representable)
    assert temporary._alive(0xFFFFFFFF)


@pytest.mark.parametrize("failure", [PermissionError, OSError, ValueError])
def test_posix_unknown_process_status_never_grants_cleanup(failure, monkeypatch):
    if sys.platform == "win32":
        # The real process test above exercises the Windows query-handle path.
        return

    def unknown(pid, signal):
        raise failure("sensitive system detail")

    monkeypatch.setattr(os, "kill", unknown)
    assert temporary._alive(os.getpid() + 100000)
