"""protocol-reference.md §2.7 - every receive-side check, driven by a failing frame.

The frames here are built by an implementation this project did not write:
Telethon's private ``MTProtoState._calc_key`` for the key schedule and its PUBLIC
``AES.encrypt_ige`` for the cipher. So `test_a_frame_the_oracle_built_decrypts` is
not a round trip through our own pair - it is our decrypt reading somebody else's
ciphertext, which is what Principle I asks for at this stage (research.md Q2:
recorded TDLib vectors cannot exist until a chat runs).

Every other test corrupts one field of that frame and asserts two things: the frame
is REFUSED, and nothing is returned. "Rejected, not repaired and not delivered with
a warning" is the constitution's wording, and the difference between a decryption
failure and a length oracle is checks 2 and 3 below.
"""

import hashlib
import os

import pytest

from telethon.crypto import AES
from telethon.network.mtprotostate import MTProtoState

from telethon_secret_chat import crypto
from telethon_secret_chat.errors import SecretChatError

KEY = bytes((i * 11 + 5) % 256 for i in range(256))
BODY = b"a TL-serialized decryptedMessageLayer would live here"


def oracle_frame(key=KEY, body=BODY, x=0, padding=None, claimed_length=None):
    """A complete §2.6 frame, built with Telethon's primitives.

    ``padding`` and ``claimed_length`` exist so a test can build a frame that is
    correctly encrypted and internally wrong - the interesting case, because a
    receiver that only checks the MAC accepts every one of them.
    """
    length = len(body) if claimed_length is None else claimed_length
    plain = (
        len(body).to_bytes(4, "little") + body
        if claimed_length is None
        else (length.to_bytes(4, "little") + body)
    )
    if padding is None:
        padding = os.urandom(-(len(plain) + 12) % 16 + 12)
    payload = plain + padding
    msg_key = hashlib.sha256(key[88 + x : 120 + x] + payload).digest()[8:24]
    aes_key, aes_iv = MTProtoState._calc_key(key, msg_key, x == 0)
    fingerprint = int.from_bytes(hashlib.sha1(key).digest()[12:20], "little", signed=True)
    return (
        fingerprint.to_bytes(8, "little", signed=True)
        + msg_key
        + AES.encrypt_ige(payload, aes_key, aes_iv)
    )


# --- the frame the oracle built must be readable ------------------------------


def test_a_frame_the_oracle_built_decrypts():
    """Principle I at this stage: Telethon encrypted it, this package reads it."""
    assert crypto.decrypt_frame(KEY, oracle_frame(), x=0) == BODY


def test_both_directions_decrypt():
    """§2.5: the originator reads incoming at ``x = 8`` and writes at ``x = 0``."""
    for x in (0, 8):
        assert crypto.decrypt_frame(KEY, oracle_frame(x=x), x=x) == BODY


# --- check 1: the key fingerprint ---------------------------------------------


def test_a_frame_for_another_key_is_refused():
    """§2.7 check 1: "key_fingerprint matches a key it holds"."""
    frame = oracle_frame()
    wrong = bytes([frame[0] ^ 0xFF]) + frame[1:]
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, wrong, x=0)


def test_a_frame_encrypted_under_a_different_key_is_refused():
    other = bytes((i * 3 + 1) % 256 for i in range(256))
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(key=other), x=0)


# --- check 2: the recomputed msg_key ------------------------------------------


def test_a_tampered_msg_key_is_refused():
    """§2.7 check 2: "you must check that msg_key is in fact equal to the 128 middle
    bits of the SHA256 hash of the decrypted message". This is the integrity check;
    without it the ciphertext is malleable."""
    frame = bytearray(oracle_frame())
    frame[8] ^= 0x01
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, bytes(frame), x=0)


def test_tampered_ciphertext_is_refused():
    frame = bytearray(oracle_frame())
    frame[-1] ^= 0x01
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, bytes(frame), x=0)


def test_decrypting_with_the_wrong_x_is_refused():
    """A frame written at ``x = 0`` and read at ``x = 8`` derives different keys, so
    the recomputed ``msg_key`` cannot match. This is what catches a side confusion -
    §1.6 calls getting the side wrong the most likely silent failure."""
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(x=0), x=8)


# --- check 3: the length prefix -----------------------------------------------


def test_a_length_prefix_beyond_the_decrypted_body_is_refused():
    """§2.7 check 3: "the 4-byte length prefix does not exceed the decrypted
    buffer". §8.2 measured the archived package testing this off by four - it
    compared against the whole buffer rather than the buffer minus the prefix."""
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(claimed_length=len(BODY) + 4096), x=0)


def test_a_length_prefix_exactly_four_too_long_is_refused():
    """The off-by-four itself: a length that fits the buffer but not the buffer
    after its own prefix. This is the case the archived package's test would pass."""
    body = BODY
    padding = os.urandom(-(4 + len(body) + 12) % 16 + 12)
    over = len(body) + len(padding)  # reaches the end of the buffer, prefix ignored
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(body=body, claimed_length=over + 1), x=0)


def test_a_negative_length_prefix_is_refused():
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(claimed_length=2**32 - 4), x=0)


# --- checks 4 and 5: padding --------------------------------------------------


def test_padding_shorter_than_twelve_is_refused():
    """§2.2: "padded with 12 to 1024 random padding bytes". Eleven is not twelve, and
    MTProto 1.0's 0-15 range is exactly what the floor exists to exclude."""
    body = b"x" * 17  # 4 + 17 + 11 = 32, correctly 16-aligned and still too little
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(body=body, padding=b"p" * 11), x=0)


def test_an_unaligned_frame_is_refused():
    """§2.7 check 5: "total decrypted length is a multiple of 16"."""
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame(padding=b"p" * 13), x=0)


def test_padding_longer_than_1024_is_ACCEPTED():
    """§2.2, and this one is deliberate.

    "**UNVERIFIED:** no consulted source says a receiver must reject padding longer
    than 1024 bytes, and the oracle itself can produce more. Treat 1024 as a
    sender-side guideline; do not add a receive-side upper bound without an interop
    test proving official clients honour it."

    TDLib rounds padding up through 1280 and beyond in 448-byte steps, so a
    receive-side ceiling would refuse messages the reference implementation
    legitimately sends. The ceiling is enforced when SENDING - see
    tests/unit/test_padding.py - and not when receiving.
    """
    big = os.urandom(-(4 + len(BODY) + 2000) % 16 + 2000)
    assert crypto.decrypt_frame(KEY, oracle_frame(padding=big), x=0) == BODY


# --- a truncated frame --------------------------------------------------------


@pytest.mark.parametrize("size", [0, 8, 23, 24, 25])
def test_a_frame_too_short_to_hold_a_header_is_refused(size):
    """The header alone is 24 bytes (§2.6), and the ciphertext after it must be a
    non-empty multiple of 16."""
    with pytest.raises(SecretChatError):
        crypto.decrypt_frame(KEY, oracle_frame()[:size], x=0)


# --- nothing leaks on the way out ---------------------------------------------


def test_a_refusal_carries_no_key_no_plaintext_and_no_ciphertext():
    """Principle IV, on the path most likely to be logged by an application."""
    frame = bytearray(oracle_frame(body=b"the launch code is 1234"))
    frame[-1] ^= 0x01
    with pytest.raises(SecretChatError) as caught:
        crypto.decrypt_frame(KEY, bytes(frame), x=0)
    text = str(caught.value) + repr(caught.value)
    assert "launch code" not in text
    assert KEY.hex()[:32] not in text.lower()
    assert bytes(frame).hex()[:32] not in text.lower()
