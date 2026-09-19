"""Diffie-Hellman parameter validation - protocol-reference.md §1.2.

Everything a client is documented to check before it derives a key, and nothing it
is not. §8.1 measured the archived package performing none of the primality work
and no range check on ``g`` at all, which is why this module exists before the one
that establishes a chat.

The order inside each function is load-bearing. Cheap structural checks run first
so a malformed parameter costs nothing, and the refusal always happens BEFORE any
modular exponentiation - a caller that validates after computing ``pow(g_b, a, p)``
has already done the work the bad value was sent to cause.

Nothing here is secret, but nothing here is quoted in an error either: a refusal
names the condition that failed, never the 2048-bit number that failed it
(Principle IV).
"""

from __future__ import annotations

import secrets
from functools import lru_cache

from .errors import ParameterRejected

__all__ = [
    "check_config",
    "check_peer_value",
    "is_probable_prime",
    "is_safe_prime",
    "generator_is_quadratic_residue",
    "value_from_bytes",
    "DH_BITS",
    "PEER_VALUE_FLOOR",
]

# §1.2: "2^2047 < p < 2^2048", so exactly 2048 bits.
DH_BITS = 2048

# §1.2: "We recommend checking that g_a and g_b are between 2^{2048-64} and
# p - 2^{2048-64} as well." Documented as a recommendation and ENFORCED by the
# oracle - TDLib's dh_check returns an error naming this exact range - so it is a
# hard check here. §1.2 makes the oracle the tie-breaker where the documentation is
# softer than the implementations.
PEER_VALUE_FLOOR = 2 ** (DH_BITS - 64)

# §1.2: "Since g is always equal to 2, 3, 4, 5, 6 or 7, this is easily done using
# quadratic reciprocity law, yielding a simple condition on p mod 4g." The table is
# transcribed from the reference; `4` carries the condition "none" because 4 is a
# perfect square and therefore a residue modulo every prime.
RESIDUE_CONDITIONS = {
    2: (8, {7}),
    3: (3, {2}),
    4: None,
    5: (5, {1, 4}),
    6: (24, {19, 23}),
    7: (7, {3, 5, 6}),
}

# The documentation permits "only 15 Miller-Rabin iterations ... with error
# probability not exceeding one billionth". Taken as stated rather than inflated:
# the parameters come from Telegram's own server, what this guards against is a
# substituted prime, and a substituted prime does not survive 15 random bases.
MILLER_RABIN_ROUNDS = 15

_SMALL_PRIMES = (2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47)


def is_probable_prime(n: int, rounds: int = MILLER_RABIN_ROUNDS) -> bool:
    """Miller-Rabin with random bases.

    Bases come from ``secrets`` rather than ``random``: the constitution bans the
    non-cryptographic module anywhere near key material, and a predictable base set
    is a set an adversary can build a composite to survive.
    """
    if n < 2:
        return False
    for small in _SMALL_PRIMES:
        if n % small == 0:
            return n == small
    d, s = n - 1, 0
    while d % 2 == 0:
        d //= 2
        s += 1
    for _ in range(rounds):
        a = secrets.randbelow(n - 3) + 2
        x = pow(a, d, n)
        if x == 1 or x == n - 1:
            continue
        for _ in range(s - 1):
            x = x * x % n
            if x == n - 1:
                break
        else:
            return False
    return True


@lru_cache(maxsize=8)
def is_safe_prime(p: int) -> bool:
    """§1.2: "both p and (p-1)/2 are prime".

    Cached because a server sends the same prime for every chat and the pair of
    2048-bit Miller-Rabin runs costs about a second. TDLib caches the same verdict
    through ``DhCallback::add_good_prime`` for the same reason.
    """
    return is_probable_prime(p) and is_probable_prime((p - 1) // 2)


def generator_is_quadratic_residue(g: int, p: int) -> bool:
    """§1.2: does ``g`` generate the subgroup of prime order ``(p-1)/2``?

    A ``g`` outside the documented set of six has no published condition, so it
    cannot be shown to generate the right subgroup and is not accepted.
    """
    if g not in RESIDUE_CONDITIONS:
        return False
    condition = RESIDUE_CONDITIONS[g]
    if condition is None:  # g = 4, a perfect square
        return True
    modulus, allowed = condition
    return p % modulus in allowed


def check_config(*, g: int, p: int, chat_id: int | None = None) -> None:
    """Validate a server-supplied ``(g, p)``. Raises ``ParameterRejected``.

    §1.1: "Executing this method before each new key generation procedure is of
    vital importance" - and checking what it returns is the other half of that.
    """
    if p.bit_length() != DH_BITS or not 2 ** (DH_BITS - 1) < p < 2**DH_BITS:
        raise ParameterRejected(
            chat_id=chat_id,
            reason="the prime is not 2048 bits: the protocol requires 2^2047 < p < 2^2048",
        )
    # Range before residue, and both before primality: the first two are
    # comparisons, the third is a second of modular arithmetic.
    if not 1 < g < p - 1:
        raise ParameterRejected(chat_id=chat_id, reason="the generator is outside 1 < g < p-1")
    if not generator_is_quadratic_residue(g, p):
        raise ParameterRejected(
            chat_id=chat_id,
            reason=(
                "the generator does not generate the subgroup of order (p-1)/2 for "
                "this prime, or is not one of the six the protocol defines"
            ),
        )
    if not is_safe_prime(p):
        raise ParameterRejected(
            chat_id=chat_id,
            reason="the prime is not a safe prime: p and (p-1)/2 must both be prime",
        )


def check_peer_value(value: int, p: int, *, chat_id: int | None = None) -> None:
    """Validate a ``g_a`` / ``g_b`` that arrived from the peer. §1.2, checks 4 and 5.

    Applies to every DH value that ever arrives, the rekey values of §4.2 and §4.3
    included - the reference is explicit that the initial exchange's limitations
    carry over to them.
    """
    if not 1 < value < p - 1:
        raise ParameterRejected(
            chat_id=chat_id, reason="the peer's public value is outside 1 < value < p-1"
        )
    if not PEER_VALUE_FLOOR <= value <= p - PEER_VALUE_FLOOR:
        raise ParameterRejected(
            chat_id=chat_id,
            reason=(
                "the peer's public value is outside 2^1984 <= value <= p - 2^1984, "
                "the range small-subgroup and edge values fall into"
            ),
        )


def value_from_bytes(raw: bytes) -> int:
    """A DH value off the wire. Big-endian, as every TL ``bytes`` carrying one is."""
    return int.from_bytes(raw, "big")
