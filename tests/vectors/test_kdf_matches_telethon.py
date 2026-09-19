"""protocol-reference.md §2.4 against an implementation this project did not write.

Constitution Principle I: "Two instances of the same wrong code agree with each
other perfectly, so a round trip through our own encrypt/decrypt pair is NOT
evidence of correctness." This file is the answer to that for the key derivation.
The oracle is Telethon's ``MTProtoState._calc_key`` - written by someone else, run
by every Telethon user on every connection, and carrying the same ``x = 0 / x = 8``
split §2.5 specifies.

Why the KDF is vendored rather than called: ``_calc_key`` is PRIVATE. A Telethon
release may change it without notice, and a cryptographic step that changes without
notice is the failure this package exists to avoid. So the ten lines live in
``crypto.py`` and the private method is used HERE, where a divergence fails loudly
and says which input produced it - research.md Q2, oracle 2.

This tier runs with no account and no network.
"""

import hashlib
import os

import pytest

from telethon.network.mtprotostate import MTProtoState

from telethon_secret_chat import crypto

# 256 bytes, the length §1.4 pads every shared key to. Fixed rather than random so a
# failure names one reproducible input; the random cases below cover the rest.
KEY = bytes((i * 7 + 13) % 256 for i in range(256))


@pytest.mark.parametrize("x", [0, 8])
def test_the_vendored_kdf_agrees_with_telethons(x):
    """§2.4, both directions. ``x = 0`` is the originator's outgoing direction."""
    msg_key = hashlib.sha256(b"pin").digest()[:16]
    assert crypto.derive_keys(KEY, msg_key, x) == MTProtoState._calc_key(KEY, msg_key, x == 0)


@pytest.mark.parametrize("x", [0, 8])
def test_they_agree_on_random_inputs(x):
    """One fixed vector proves the splices are not transposed; a hundred random ones
    prove it for every byte position, which is where an off-by-one in a ``substr``
    hides."""
    for _ in range(100):
        key, msg_key = os.urandom(256), os.urandom(16)
        assert crypto.derive_keys(key, msg_key, x) == MTProtoState._calc_key(key, msg_key, x == 0)


def test_the_two_directions_do_not_share_a_key():
    """§2.5: "Both x values index the same 256-byte shared key, so the two
    directions never share an aes_key." A KDF that ignored ``x`` would pass every
    comparison above if the oracle were called with the same argument."""
    msg_key = os.urandom(16)
    assert crypto.derive_keys(KEY, msg_key, 0) != crypto.derive_keys(KEY, msg_key, 8)


def test_an_x_outside_the_protocol_is_refused():
    """§2.5 defines two values. A third would silently index the key elsewhere."""
    with pytest.raises(ValueError):
        crypto.derive_keys(KEY, os.urandom(16), 4)


def test_the_key_and_iv_are_the_sizes_aes_256_ige_needs():
    aes_key, aes_iv = crypto.derive_keys(KEY, os.urandom(16), 0)
    assert len(aes_key) == 32 and len(aes_iv) == 32


def test_msg_key_is_the_middle_128_bits_of_the_documented_hash():
    """§2.3, recomputed here from the quoted formula rather than from the module:

        msg_key_large = SHA256 (substr (key, 88+x, 32) + plaintext + random_padding)
        msg_key       = substr (msg_key_large, 8, 16)

    Two properties are what distinguish this from MTProto 1.0 and are what this
    asserts: the padding IS inside the hash, and 32 bytes of the shared key are too.
    """
    for x in (0, 8):
        payload = os.urandom(64)
        expected = hashlib.sha256(KEY[88 + x : 120 + x] + payload).digest()[8:24]
        assert crypto.compute_msg_key(KEY, payload, x) == expected
        assert len(expected) == 16


def test_msg_key_covers_the_padding_too():
    """If the padding were excluded from the hash, two messages differing only in
    their padding would carry the same ``msg_key`` - and the integrity check would
    not cover the bytes an attacker is most free to choose."""
    body = os.urandom(32)
    assert crypto.compute_msg_key(KEY, body + b"\x00" * 16, 0) != crypto.compute_msg_key(
        KEY, body + b"\x01" * 16, 0
    )


def test_the_fingerprint_is_the_last_64_bits_of_sha1_little_endian():
    """§1.5: "equal to the 64 last bits of SHA1 (key)", read as a little-endian
    signed int64 - TDLib's ``as<int64>(auth_key_sha1.raw + 12)``. Note 1 on that
    sentence: SHA1 even for MTProto 2.0 secret chats."""
    expected = int.from_bytes(hashlib.sha1(KEY).digest()[12:20], "little", signed=True)
    assert crypto.key_fingerprint(KEY) == expected
