"""MTProto 2.0 key derivation and the message frame - protocol-reference.md §2.

Serialization and length prefix (§2.1), padding (§2.2), ``msg_key`` (§2.3), the
``aes_key``/``aes_iv`` KDF (§2.4), the ``x = 0 / x = 8`` split (§2.5), the outer
frame (§2.6) and the receive-side checks (§2.7).

Two decisions worth stating once, here, rather than re-deriving at each call site.

**The KDF is vendored, the cipher is not.** §2.4 is ten lines and fully specified,
so it lives here - Telethon's ``MTProtoState._calc_key`` is PRIVATE and a
cryptographic step that can change on a patch release is not a step to depend on.
It is used as the TEST oracle instead (tests/vectors/test_kdf_matches_telethon.py),
so the two are compared on every run. AES-IGE is the opposite case:
``telethon.crypto.AES`` is PUBLIC, it is the primitive Telethon's own transport
runs on, and writing a cipher loop here is precisely what research.md Q1 decided
against.

**MTProto 1.0 does not exist in this module.** FR-017 and Principle II. There is no
``version`` parameter, no fallback, and no path that produces or accepts a 1.0
frame - §8.2 measured the archived package defaulting to 1.0 for the first
messages of every chat and permanently downgrading on any decryption exception,
which is a downgrade an attacker can cause by corrupting one byte.
"""

from __future__ import annotations

import hashlib
import hmac
import os

from telethon.crypto import AES

from .errors import MessageRejected

__all__ = [
    "KEY_LENGTH",
    "HEADER_LENGTH",
    "MIN_PADDING",
    "MAX_PADDING",
    "key_fingerprint",
    "compute_msg_key",
    "derive_keys",
    "pad_payload",
    "encrypt_frame",
    "decrypt_frame",
]

# §1.4: "If key length < 256 bytes, add several leading zero bytes as padding so
# that the key is exactly 256 bytes long."
KEY_LENGTH = 256

# §2.6: key_fingerprint (8) || msg_key (16), then the ciphertext.
HEADER_LENGTH = 24

# §2.2: "padded with 12 to 1024 random padding bytes".
MIN_PADDING = 12
MAX_PADDING = 1024

BLOCK = 16


def key_fingerprint(shared_key: bytes) -> int:
    """§1.5: "equal to the 64 last bits of SHA1 (key)".

    Read as a little-endian signed int64 - TDLib's ``calc_key_id`` returns
    ``as<int64>(auth_key_sha1.raw + 12)``. The same value is the ``auth_key_id``
    prefixed to every frame (§2.6), so one routine serves both uses.

    Note 1 on the documented sentence, and it is easy to get wrong: SHA1 is used
    here even for MTProto 2.0 secret chats.
    """
    return int.from_bytes(hashlib.sha1(shared_key).digest()[12:20], "little", signed=True)


def _check_x(x: int) -> None:
    """§2.5 defines exactly two values. A third would index the shared key somewhere
    the protocol never names, and would do it silently."""
    if x not in (0, 8):
        raise ValueError("x must be 0 or 8: protocol-reference.md §2.5")


def compute_msg_key(shared_key: bytes, payload: bytes, x: int) -> bytes:
    """§2.3, verbatim::

        msg_key_large = SHA256 (substr (key, 88+x, 32) + plaintext + random_padding)
        msg_key       = substr (msg_key_large, 8, 16)

    ``payload`` is the length prefix, the body AND the padding - all three are
    inside the hash. That, and the 32 bytes of shared key in front of them, are the
    two properties that distinguish this from MTProto 1.0; losing either turns the
    integrity check into something an attacker can satisfy.
    """
    _check_x(x)
    return hashlib.sha256(shared_key[88 + x : 120 + x] + payload).digest()[8:24]


def derive_keys(shared_key: bytes, msg_key: bytes, x: int) -> tuple[bytes, bytes]:
    """§2.4, verbatim::

        sha256_a = SHA256 (msg_key + substr (key, x, 36));
        sha256_b = SHA256 (substr (key, 40+x, 36) + msg_key);
        aes_key  = substr (sha256_a, 0, 8) + substr (sha256_b, 8, 16) + substr (sha256_a, 24, 8);
        aes_iv   = substr (sha256_b, 0, 8) + substr (sha256_a, 8, 16) + substr (sha256_b, 24, 8);

    The two are mirror images, taking ``a``/``b`` in the opposite order. Pinned
    against Telethon's implementation on every test run.
    """
    _check_x(x)
    sha256_a = hashlib.sha256(msg_key + shared_key[x : x + 36]).digest()
    sha256_b = hashlib.sha256(shared_key[x + 40 : x + 76] + msg_key).digest()
    aes_key = sha256_a[:8] + sha256_b[8:24] + sha256_a[24:32]
    aes_iv = sha256_b[:8] + sha256_a[8:24] + sha256_b[24:32]
    return aes_key, aes_iv


def pad_payload(payload: bytes) -> bytes:
    """§2.2: pad to a multiple of 16 with 12 to 1024 bytes from a secure source.

    The smallest conforming choice, 12 to 27 bytes. TDLib pads more generously -
    rounding up through 64, 128, 256 and beyond - which hides message length a
    little; that is a sender's privacy choice rather than a protocol requirement,
    and this package takes the documented minimum so the rule under test is the
    rule the reference states.

    ``os.urandom`` and nothing else. §8.3 recorded the archived package choosing its
    padding LENGTH with ``random.randint``, and the constitution names ``random`` in
    any padding path as forbidden.
    """
    length = MIN_PADDING + (-(len(payload) + MIN_PADDING) % BLOCK)
    return payload + os.urandom(length)


def encrypt_frame(shared_key: bytes, body: bytes, x: int) -> bytes:
    """Body in, a complete §2.6 frame out.

    ``body`` is the serialized ``decryptedMessageLayer`` (§3.1); §2.1 prepends "4
    bytes containing the array length not counting these 4 bytes", §2.2 pads, §2.3
    keys the hash and §2.6 prefixes the fingerprint and the ``msg_key``.
    """
    _check_x(x)
    payload = pad_payload(len(body).to_bytes(4, "little") + body)
    msg_key = compute_msg_key(shared_key, payload, x)
    aes_key, aes_iv = derive_keys(shared_key, msg_key, x)
    fingerprint = key_fingerprint(shared_key).to_bytes(8, "little", signed=True)
    return fingerprint + msg_key + AES.encrypt_ige(payload, aes_key, aes_iv)


def decrypt_frame(shared_key: bytes, frame: bytes, x: int, *, chat_id: int | None = None) -> bytes:
    """§2.7's checks, in the documented order. Returns the body, or raises.

    "A received message failing any check - fingerprint, ``msg_key``, sequence
    number, layer - MUST be rejected. It MUST NOT be repaired, guessed at, or
    delivered with a warning" (constitution, Secret Material Handling). So every
    branch below raises and none of them returns a partial result.

    The sequence-number and layer checks are NOT here: they need the decoded object
    and the chat's counters, and they live in ``sequence.py`` (§3.4-§3.6).
    """
    _check_x(x)

    # Check 1: the fingerprint names a key we hold. Cheap, and it is what lets a
    # receiver pick between the current and the previous key during a rekey (§4.5).
    if len(frame) < HEADER_LENGTH + BLOCK:
        raise MessageRejected(chat_id=chat_id, reason="the frame is too short to hold a message")
    expected = key_fingerprint(shared_key).to_bytes(8, "little", signed=True)
    if not hmac.compare_digest(frame[:8], expected):
        raise MessageRejected(
            chat_id=chat_id, reason="the key fingerprint does not name a key this chat holds"
        )

    ciphertext = frame[HEADER_LENGTH:]
    if len(ciphertext) % BLOCK:
        raise MessageRejected(
            chat_id=chat_id, reason="the ciphertext is not a whole number of cipher blocks"
        )

    msg_key = frame[8:HEADER_LENGTH]
    aes_key, aes_iv = derive_keys(shared_key, msg_key, x)
    decrypted = AES.decrypt_ige(ciphertext, aes_key, aes_iv)

    # Check 2: the integrity check. Recomputed over the WHOLE decrypted buffer,
    # padding included, because that is what §2.3 hashes. Constant-time, because a
    # comparison that returns early on the first wrong byte is a comparison an
    # attacker can search.
    if not hmac.compare_digest(compute_msg_key(shared_key, decrypted, x), msg_key):
        raise MessageRejected(
            chat_id=chat_id,
            reason="the recomputed message key does not match the one received",
        )

    # Check 5 before 3 and 4: alignment is a property of the buffer, and the two
    # length checks below are only meaningful once it holds.
    if len(decrypted) % BLOCK:
        raise MessageRejected(
            chat_id=chat_id, reason="the decrypted length is not a multiple of 16"
        )

    # Check 3: the length prefix is inside the buffer - AFTER its own four bytes.
    # §8.2 measured the archived package comparing against the whole buffer instead,
    # which accepts a length four bytes too long. That difference is the one §2.7
    # calls "the difference between a decryption failure and a buffer-length oracle".
    declared = int.from_bytes(decrypted[:4], "little")
    if declared > len(decrypted) - 4:
        raise MessageRejected(
            chat_id=chat_id, reason="the declared body length runs past the decrypted buffer"
        )

    # Check 4: padding is at least 12 (§2.2). There is deliberately NO upper bound
    # here. §2.2 marks a receive-side ceiling UNVERIFIED - "the oracle itself can
    # produce more" - and TDLib rounds padding up through 1280 and beyond, so a
    # 1024 ceiling on receive would refuse messages the reference implementation
    # legitimately sends. The ceiling is enforced when sending, in `pad_payload`.
    if len(decrypted) - 4 - declared < MIN_PADDING:
        raise MessageRejected(
            chat_id=chat_id, reason="the padding is shorter than the 12 bytes required"
        )

    return decrypted[4 : 4 + declared]
