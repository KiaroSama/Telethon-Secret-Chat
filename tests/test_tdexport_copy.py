"""The shared port and every Desktop resource stay identical to the verified handoff."""

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_exporter_copy_matches_verified_source():
    manifest = json.loads(
        (ROOT / "tests/fixtures/tdexport-source.json").read_text(encoding="utf-8")
    )
    package = ROOT / "telethon_secret_chat/tdexport"
    expected = manifest["sha256"]
    actual = {
        str(p.relative_to(package)).replace("\\", "/")
        for p in package.rglob("*")
        if p.is_file() and "__pycache__" not in p.parts
    }
    assert actual == set(expected)
    for name, digest in expected.items():
        assert hashlib.sha256((package / name).read_bytes()).hexdigest() == digest, name
    for name in ("assets/css/style.css", "assets/js/script.js"):
        assert b"\r\n" in (package / name).read_bytes()


def test_export_assets_are_declared_for_wheels():
    import tomllib

    config = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    assert config["tool"]["setuptools"]["package-data"]["telethon_secret_chat.tdexport"] == [
        "assets/css/*",
        "assets/js/*",
        "assets/images/*",
    ]
    notice = (ROOT / "NOTICE").read_text(encoding="utf-8")
    assert "TELEGRAM DESKTOP EXPORT" in notice
    assert "Copyright (C) 2004, 2005 Daniel M. Duley" in notice
    assert "qt_icc.py" in notice
