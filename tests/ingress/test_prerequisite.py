"""Public prerequisite preparation rejects unknown source without partial output."""

import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = pytest.mark.timeout(10)
ROOT = Path(__file__).resolve().parents[2]


def test_unknown_upstream_archive_fails_without_output(tmp_path):
    source = tmp_path / "unknown.whl"
    source.write_bytes(b"not the pinned public upstream wheel")
    output = tmp_path / "wheels"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "tools/build_telethon.py"),
            "--archive",
            str(source),
            "--output",
            str(output),
        ],
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=5,
    )
    assert result.returncode == 2
    assert "upstream archive checksum does not match" in result.stderr
    assert not list(output.glob("*.whl"))
