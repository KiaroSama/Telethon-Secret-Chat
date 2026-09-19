"""protocol-reference.md §1.5 and §1.4 - the shared key and the value both ends show.

The fingerprint is the one number a user can compare between two devices, and §1.4
makes it the check that the exchange worked: "If the fingerprint for the received
key is identical to the one that was passed to encryptedChat, incoming messages can
be sent and processed. Otherwise, messages.discardEncryption must be executed and
the user notified."

So the tests here are about the two ways it goes wrong silently: a key padded to the
wrong width, and an integer read in the wrong byte order. Both produce a fingerprint
that is stable, plausible, and different from the peer's.
"""

import hashlib
import os

import pytest

from telethon_secret_chat import crypto, handshake
from telethon_secret_chat.errors import ParameterRejected

from .dh_material import SAFE_PRIME

G = 2


# --- §1.5 the fingerprint itself ----------------------------------------------


def test_the_fingerprint_is_sha1_of_the_key_last_eight_bytes_little_endian():
    """§1.5: "equal to the 64 last bits of SHA1 (key)", and TDLib's ``calc_key_id``
    reads those bytes as a little-endian signed int64.

    Note 1 on that sentence, and the easy mistake: SHA1 is used here even for
    MTProto 2.0 secret chats."""
    key = bytes(range(256))
    expected = int.from_bytes(hashlib.sha1(key).digest()[12:20], "little", signed=True)
    assert crypto.key_fingerprint(key) == expected


def test_the_fingerprint_fits_a_signed_int64():
    """It travels as a TL ``long``. A value computed unsigned would not round-trip."""
    for _ in range(50):
        value = crypto.key_fingerprint(os.urandom(256))
        assert -(2**63) <= value < 2**63


def test_a_different_key_gives_a_different_fingerprint():
    assert crypto.key_fingerprint(b"\x00" * 256) != crypto.key_fingerprint(b"\x01" * 256)


# --- §1.4 the shared key ------------------------------------------------------


def test_both_sides_derive_the_same_key_and_fingerprint():
    """§1.6: A computes ``g_b^a mod p``, B computes ``g_a^b mod p``. The whole
    exchange is this one equality, and the fingerprint is how the two ends check it
    without exchanging the key."""
    a = handshake.generate_secret()
    b = handshake.generate_secret()
    g_a = handshake.public_value(G, a, SAFE_PRIME)
    g_b = handshake.public_value(G, b, SAFE_PRIME)

    key_a = handshake.shared_key(peer_value=g_b, secret=a, p=SAFE_PRIME)
    key_b = handshake.shared_key(peer_value=g_a, secret=b, p=SAFE_PRIME)

    assert key_a == key_b
    assert crypto.key_fingerprint(key_a) == crypto.key_fingerprint(key_b)


def test_the_shared_key_is_always_256_bytes():
    """§1.4: "If key length < 256 bytes, add several leading zero bytes as padding so
    that the key is exactly 256 bytes long." TDLib does it as
    ``get_g_ab().to_binary(2048 / 8)`` - fixed width, big-endian, left-padded.

    A key that is sometimes 255 bytes is a key whose §2.4 substrings are all shifted
    by one, which fails only for the fraction of exchanges that land on a short
    value - roughly one in 256, i.e. rarely enough to reach production.
    """
    # A VALID peer value with an exponent of zero: the shared value is 1, so the
    # result is 255 zero bytes and a 1. Reaching this branch by sending a SMALL
    # g_b instead is impossible by design - §1.2 refuses it, which is what
    # `test_deriving_a_key_validates_the_peer_value_first` below asserts.
    short = handshake.shared_key(peer_value=SAFE_PRIME // 2, secret=0, p=SAFE_PRIME)
    assert len(short) == 256

    a, b = handshake.generate_secret(), handshake.generate_secret()
    full = handshake.shared_key(
        peer_value=handshake.public_value(G, b, SAFE_PRIME), secret=a, p=SAFE_PRIME
    )
    assert len(full) == 256


def test_the_padding_is_on_the_LEFT():
    """Right-padding would also produce 256 bytes, and a fingerprint, and a chat
    that decrypts nothing - every §2.3 and §2.4 substring is an offset into this
    buffer, so the whole key schedule shifts."""
    key = handshake.shared_key(peer_value=SAFE_PRIME // 2, secret=0, p=SAFE_PRIME)
    assert key[-1] == 1, "the value must sit at the END of the buffer"
    assert key[:255] == b"\x00" * 255, "the padding must be at the FRONT"


def test_the_secret_is_2048_bits_from_a_secure_source():
    """§1.3: "Client A computes a 2048-bit number a (using sufficient entropy...)"."""
    values = [handshake.generate_secret() for _ in range(20)]
    assert all(v.bit_length() > 2000 for v in values)
    assert len(set(values)) == 20


def test_deriving_a_key_validates_the_peer_value_first():
    """§1.2 applies to every DH value that arrives. A ``g_b`` of 1 makes the shared
    key 1, and the check must run before the exponentiation rather than after it."""
    with pytest.raises(ParameterRejected):
        handshake.shared_key(peer_value=1, secret=handshake.generate_secret(), p=SAFE_PRIME)


def test_the_fingerprint_comparison_refuses_a_mismatch():
    """§1.4: A compares its own fingerprint against the one in ``encryptedChat``, and
    on a mismatch "messages.discardEncryption must be executed and the user
    notified" - not a warning, not a retry."""
    key = os.urandom(256)
    handshake.verify_fingerprint(chat_id=5, key=key, claimed=crypto.key_fingerprint(key))
    with pytest.raises(ParameterRejected):
        handshake.verify_fingerprint(chat_id=5, key=key, claimed=crypto.key_fingerprint(key) ^ 1)


def test_a_fingerprint_mismatch_does_not_report_either_value():
    """Principle IV. The fingerprint is derived from the key, and a message carrying
    both the expected and received values is a 128-bit window onto SHA1(key)."""
    key = os.urandom(256)
    claimed = crypto.key_fingerprint(key) ^ 1
    with pytest.raises(ParameterRejected) as caught:
        handshake.verify_fingerprint(chat_id=5, key=key, claimed=claimed)
    text = str(caught.value) + repr(caught.value)
    assert str(claimed) not in text
    assert str(crypto.key_fingerprint(key)) not in text
    assert key.hex()[:32] not in text.lower()
