"""Installed-wheel prerequisite provenance and fail-closed activation contract."""

import hashlib
import importlib.metadata
import json
from pathlib import Path

from telethon import TelegramClient
from telethon.sessions import MemorySession
from telethon._updates.durable_ingress import API_VERSION
from telethon_secret_chat import (
    DurableIngress,
    EncryptedIngressStorage,
    IngressLimits,
    StrongSecretChatManager,
)


def main():
    distribution = importlib.metadata.distribution("telethon")
    assert distribution.version == "1.45.0+kiaro.ingress1"
    raw = distribution.read_text("kiaro-provenance.json")
    assert raw is not None
    receipt = json.loads(raw)
    assert receipt["sha256"] == "339e24c83aedc9f12cbe389264627edc49779e4289c535aa90aafc4ce62f7972"
    assert receipt["commit"] == "bad095bd5b3dc9f852cfbeeba300a421169c9b1e"
    for name, expected in receipt["module_hashes"].items():
        source = Path(distribution.locate_file(name))
        assert hashlib.sha256(source.read_bytes()).hexdigest() == expected, name
    assert API_VERSION == DurableIngress.api_version == 1
    from telethon_secret_chat.ingress_provenance import verify_prerequisite

    verify_prerequisite()
    assert callable(DurableIngress.prepare)
    assert callable(EncryptedIngressStorage.close)
    assert callable(StrongSecretChatManager.pending_deliveries)
    assert IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600).records == 32
    client = TelegramClient(MemorySession(), 1, "0" * 32)
    # No connect/login: creating a caller-owned client proves the installed public seam.
    assert callable(client.configure_durable_ingress)
    assert not client.is_connected()
    try:
        client.configure_durable_ingress(object())
    except ValueError:
        pass
    else:
        raise AssertionError("incomplete ingress contracts must refuse activation")
    import asyncio
    import secrets
    import tempfile
    from telethon.crypto import AuthKey
    from telethon._updates import MessageBox, SessionState
    import logging
    import datetime

    client.session.set_dc(2, "127.0.0.1", 443)
    client.session.auth_key = AuthKey(bytes(range(256)))
    binding = dict(
        user_id=123,
        auth_key_id=client.session.auth_key.key_id,
        dc_id=2,
        generation="wheel-fixture",
    )

    async def prepare_fixture(path):
        store = EncryptedIngressStorage(
            path, key=secrets.token_bytes(32), binding=binding, max_bytes=1024 * 1024
        )
        try:
            ingress = DurableIngress(
                store, limits=IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
            )
            box = MessageBox(logging.getLogger("wheel-fixture"))
            box.load(
                SessionState(
                    123,
                    2,
                    False,
                    10,
                    40,
                    int(datetime.datetime.now(datetime.timezone.utc).timestamp()),
                    1,
                    None,
                ),
                [],
            )
            await ingress.prepare(client, baseline=box.session_state())
            assert client._durable_ingress is ingress and not client.is_connected()
            await ingress.stop()
        finally:
            store.close()

    with tempfile.TemporaryDirectory(prefix="ingress-wheel-fixture-") as folder:
        asyncio.run(prepare_fixture(Path(folder) / "custody.bin"))
    print("Installed prerequisite provenance and public preparation verified; no account used")


if __name__ == "__main__":
    main()
