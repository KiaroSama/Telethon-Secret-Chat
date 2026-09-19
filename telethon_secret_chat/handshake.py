"""Chat establishment - protocol-reference.md §1.3-§1.6.

The arithmetic of the exchange, with no network in it: the secret, the public value,
the shared key and the fingerprint comparison. The calls that carry them
(``messages.requestEncryption``, ``messages.acceptEncryption``) live in
``manager.py``, because what is worth testing without an account is this.

Every function that touches a value from the peer validates it through ``dh`` FIRST.
§1.2's checks "apply to every DH value that ever arrives, rekey values of §4
included", and a ``g_b`` of 1 makes the shared key 1 - so validating after the
exponentiation would be validating after the damage.
"""

from __future__ import annotations

import secrets

from . import dh
from .crypto import KEY_LENGTH, key_fingerprint
from .errors import ParameterRejected

__all__ = ["generate_secret", "public_value", "shared_key", "verify_fingerprint", "SECRET_BITS"]

# §1.3: "Client A computes a 2048-bit number a (using sufficient entropy or the
# server's random; see above)".
SECRET_BITS = 2048


def generate_secret() -> int:
    """The private exponent, from the OS CSPRNG.

    §1.1 permits mixing in the server's ``random``, and warns that "using the
    server's random sequence in its raw form may be unsafe, it must be combined with
    a client sequence". This package asks the server for none (``random_length = 0``)
    and uses a local CSPRNG only, which the same paragraph says "avoids the hazard
    entirely" - there is nothing to combine, so there is no way to combine it wrong.
    """
    # The top bit is set so the value is genuinely 2048 bits rather than "up to".
    return secrets.randbits(SECRET_BITS) | (1 << (SECRET_BITS - 1))


def public_value(g: int, secret: int, p: int) -> int:
    """``g_a = g^a mod p`` (§1.3), or ``g_b = g^b mod p`` (§1.4)."""
    return pow(g, secret, p)


def shared_key(*, peer_value: int, secret: int, p: int, chat_id: int | None = None) -> bytes:
    """§1.4: ``key = pow(g_a, b) mod dh_prime``, padded to exactly 256 bytes.

    "If key length < 256 bytes, add several leading zero bytes as padding so that the
    key is exactly 256 bytes long." TDLib does it as ``get_g_ab().to_binary(2048/8)``
    - fixed width, big-endian, LEFT-padded.

    The width is not cosmetic: every substring in §2.3 and §2.4 is an offset into
    this buffer, so a key that is occasionally 255 bytes derives different AES keys
    for roughly one exchange in 256 - often enough to reach production, rare enough
    to look like a peer problem.
    """
    dh.check_peer_value(peer_value, p, chat_id=chat_id)
    return pow(peer_value, secret, p).to_bytes(KEY_LENGTH, "big")


def verify_fingerprint(*, chat_id: int, key: bytes, claimed: int) -> None:
    """§1.4: "If the fingerprint for the received key is identical to the one that
    was passed to encryptedChat, incoming messages can be sent and processed.
    Otherwise, messages.discardEncryption must be executed and the user notified."

    Neither value appears in the error. §1.5 Note 2 calls the fingerprint "a sanity
    check ... to detect bugs when developing client software", but it is still 64
    bits of ``SHA1(key)``, and an error printing both the expected and the received
    value hands out 128 bits of it.
    """
    if key_fingerprint(key) != claimed:
        raise ParameterRejected(
            chat_id=chat_id,
            reason=(
                "the key fingerprint the peer published does not match the key this "
                "side derived: the exchange did not produce a shared key and the chat "
                "must be discarded"
            ),
        )
