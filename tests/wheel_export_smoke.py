"""Run from outside the checkout with the installed wheel interpreter; no account."""

import hashlib
import json
import tempfile
from pathlib import Path

from telethon.tl import types

from telethon_secret_chat.tdexport.html_and_json import HtmlAndJsonWriter
from telethon_secret_chat.tdexport.html_writer import HtmlWriter
from telethon_secret_chat.tdexport.json_writer import JsonWriter
from telethon_secret_chat.tdexport.secret_saved import export_saved_chat
from telethon_secret_chat.tdexport.settings import Format, Settings
import telethon_secret_chat.tdexport as package


def main(manifest_path):
    expected = json.loads(Path(manifest_path).read_text(encoding="utf-8"))["sha256"]
    installed = Path(package.__file__).parent
    for name, digest in expected.items():
        assert hashlib.sha256((installed / name).read_bytes()).hexdigest() == digest, name
    with tempfile.TemporaryDirectory(prefix="installed export space ") as folder:
        result = export_saved_chat(
            [
                {
                    "type": "message",
                    "id": 42,
                    "date": 1_700_000_000,
                    "out": True,
                    "text": "wheel فارسی",
                    "entities": [],
                    "media": None,
                }
            ],
            Settings(path=folder, format=Format.HtmlAndJson),
            HtmlAndJsonWriter(HtmlWriter(), JsonWriter()),
            types.User(id=1, is_self=True, first_name="Me"),
            types.User(id=2, first_name="Peer"),
        )
        output = Path(result.path)
        assert (
            json.loads((output / "result.json").read_text(encoding="utf-8"))["messages"][0]["text"]
            == "wheel فارسی"
        )
        assert "wheel فارسی" in (output / "messages.html").read_text(encoding="utf-8")
        for name in (
            "assets/css/style.css",
            "assets/js/script.js",
            "assets/images/media_photo.png",
        ):
            target = output / name.removeprefix("assets/")
            assert hashlib.sha256(target.read_bytes()).hexdigest() == expected[name]
    print("Installed-wheel export and all shared source hashes verified")


if __name__ == "__main__":
    import sys

    main(sys.argv[1])
