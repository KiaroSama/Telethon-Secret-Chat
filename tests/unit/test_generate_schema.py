"""The schema generator never half-writes and can check without writing (spec 002 FR-008)."""

import importlib.util
import subprocess
from pathlib import Path

import pytest

spec = importlib.util.spec_from_file_location(
    "generate_schema", Path(__file__).resolve().parents[2] / "tools/generate_schema.py"
)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)

# Two published declarations: enough for the whole pipeline, cheap to format.
SCHEMA = (
    "decryptedMessageMediaEmpty#89f5c4a = DecryptedMessageMedia;\n"
    "decryptedMessageActionNoop#a82fdd63 = DecryptedMessageAction;\n"
)


@pytest.fixture
def sandbox(tmp_path, monkeypatch):
    schema = tmp_path / "end-to-end.tl"
    schema.write_text(SCHEMA, encoding="utf-8")
    target = tmp_path / "secret_tl.py"
    monkeypatch.setattr(generator, "SCHEMA", schema)
    monkeypatch.setattr(generator, "TARGET", target)
    monkeypatch.setattr(generator, "ROOT", tmp_path)
    return tmp_path, target


def test_a_formatter_failure_leaves_the_tracked_module_untouched(sandbox, monkeypatch):
    directory, target = sandbox
    target.write_bytes(b"# the previous generated module\n")

    def fails(*args, **kwargs):
        raise subprocess.CalledProcessError(1, args[0])

    monkeypatch.setattr(generator.subprocess, "run", fails)
    with pytest.raises(subprocess.CalledProcessError):
        generator.main([])

    assert target.read_bytes() == b"# the previous generated module\n"
    assert sorted(p.name for p in directory.iterdir()) == ["end-to-end.tl", "secret_tl.py"]


def test_check_passes_on_a_match_and_fails_on_drift_without_writing(sandbox):
    directory, target = sandbox
    assert generator.main([]) == 0
    generated = target.read_bytes()

    assert generator.main(["--check"]) == 0
    target.write_bytes(generated + b"# drift\n")
    assert generator.main(["--check"]) == 1

    assert target.read_bytes() == generated + b"# drift\n"
    assert sorted(p.name for p in directory.iterdir()) == ["end-to-end.tl", "secret_tl.py"]


def test_check_fails_when_the_module_is_missing(sandbox):
    directory, target = sandbox
    assert generator.main(["--check"]) == 1
    assert not target.exists()
