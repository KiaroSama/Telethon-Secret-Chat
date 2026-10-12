"""Verify the installed prerequisite, not merely a spoofable version string."""

import hashlib
import importlib
import importlib.metadata
import json
from pathlib import Path

from .ingress_store import IngressRefused

VERSION = "1.45.0+kiaro.ingress1"
UPSTREAM = "339e24c83aedc9f12cbe389264627edc49779e4289c535aa90aafc4ce62f7972"
EXPECTED_MODULE_HASHES = {
    "telethon/client/telegrambaseclient.py": "5038c4ab18db8b71b49d6b5c3f3285c49791ed3779c5ddba1b4ab241b7b3dea6",
    "telethon/client/updates.py": "50679ecbbc81e67d2175bc96b8d61fda3941fbcd82ae5fca281f92ba8b339a7d",
    "telethon/client/users.py": "fdb2d69f5b1716e80b898a95e5f30053872c352a3bba6211674df1d51b7bdeeb",
    "telethon/client/auth.py": "355a8578ff23f81138da77c7564a0338e9edd0dca5452a1b1b6010f436772707",
    "telethon/_updates/durable_ingress.py": "5d7feb1491f88cb55faa7c6ebdbd2f69e488078d1105a4d886a70929ea99a8a9",
}


def verify_prerequisite():
    try:
        distribution = importlib.metadata.distribution("telethon")
        receipt = json.loads(distribution.read_text("kiaro-provenance.json") or "null")
        if (
            distribution.version != VERSION
            or not isinstance(receipt, dict)
            or receipt["sha256"] != UPSTREAM
            or receipt["extension_api"] != 1
        ):
            raise IngressRefused()
        hashes = receipt["module_hashes"]
        required = {
            "telethon/client/updates.py",
            "telethon/client/users.py",
            "telethon/client/telegrambaseclient.py",
            "telethon/client/auth.py",
            "telethon/_updates/durable_ingress.py",
        }
        if (
            not isinstance(hashes, dict)
            or any(hashes.get(name) != value for name, value in EXPECTED_MODULE_HASHES.items())
            or not required.issubset(hashes)
        ):
            raise IngressRefused()
        for name in required:
            path = Path(str(distribution.locate_file(name)))
            module = importlib.import_module(name[:-3].replace("/", "."))
            if (
                Path(str(module.__file__)).resolve() != path.resolve()
                or hashlib.sha256(path.read_bytes()).hexdigest() != hashes[name]
            ):
                raise IngressRefused()
    except Exception:
        raise IngressRefused() from None
