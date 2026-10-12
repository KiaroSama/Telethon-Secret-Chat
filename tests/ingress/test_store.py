"""Protected durable custody public boundary, not an archive."""

import secrets

import pytest
import telethon_secret_chat as api

pytestmark = pytest.mark.timeout(10)


def test_protected_checkpoint_roundtrip_and_exclusive_owner(tmp_path):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "custody.bin"
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    try:
        with store.transaction():
            store.save_setting("checkpoint", {"qts": 40, "marker": "plaintext fixture sentinel"})
        assert b"plaintext fixture sentinel" not in path.read_bytes()
        with pytest.raises(api.IngressRefused):
            api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    finally:
        store.close()
    restored = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    try:
        assert restored.load_setting("checkpoint")["qts"] == 40
        before = path.read_bytes()
        with pytest.raises(api.IngressRefused):
            with restored.transaction():
                restored.save_setting("checkpoint", {"qts": 41, "oversize": "x" * 1024 * 1024})
        assert restored.load_setting("checkpoint")["qts"] == 40
        assert path.read_bytes() == before
    finally:
        restored.close()


@pytest.mark.parametrize("change", ["key", "binding", "ciphertext"])
def test_protected_store_refuses_wrong_key_binding_or_corrupt_ciphertext(tmp_path, change):
    key = secrets.token_bytes(32)
    binding = dict(user_id=123, auth_key_id=456, dc_id=2, generation="fixture-generation")
    path = tmp_path / "custody.bin"
    store = api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)
    store.save_setting("held", {"text": "public fixture"})
    store.close()
    if change == "key":
        key = secrets.token_bytes(32)
    elif change == "binding":
        binding["auth_key_id"] = 457
    else:
        value = bytearray(path.read_bytes())
        value[-1] ^= 1
        path.write_bytes(value)
    with pytest.raises(api.IngressRefused):
        api.EncryptedIngressStorage(path, key=key, binding=binding, max_bytes=1024 * 1024)


def test_established_aead_matches_published_nist_empty_plaintext_vector():
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    # NIST SP800-38D zero AES-128 key/96-bit IV, empty plaintext and AAD.
    assert (
        AESGCM(bytes(16)).encrypt(bytes(12), b"", b"").hex() == "58e2fccefa7e3061367f1d57a4e7455a"
    )
