"""protocol-reference.md §1.2, the peer's public value - checked BEFORE the shared key.

Two bounds, and the reference is explicit that both apply to every DH value that
ever arrives, the rekey values of §4 included:

1. ``1 < g_a, g_b < p-1`` - required by the documentation.
2. ``2^1984 <= g_a, g_b <= p - 2^1984`` - "recommended" by the documentation and
   ENFORCED by TDLib, which returns an error naming exactly this range. §1.2 records
   the oracle as the tie-breaker where the documentation is softer, so it is a hard
   check here too.

The ordering is the point of the file. A value that fails these must be refused
before ``pow(g_b, a, p)`` runs: a ``g_b`` of 1 makes the shared key 1, and a chat
whose key an attacker chose is worse than no chat.
"""

import builtins

import pytest

from telethon_secret_chat import dh
from telethon_secret_chat.errors import ParameterRejected

from .dh_material import SAFE_PRIME

LOW_BOUND = 2 ** (2048 - 64)


def test_a_normal_peer_value_is_accepted():
    dh.check_peer_value(SAFE_PRIME // 2, SAFE_PRIME)


@pytest.mark.parametrize("value", [0, 1, -1])
def test_a_value_at_or_below_one_is_refused(value):
    with pytest.raises(ParameterRejected):
        dh.check_peer_value(value, SAFE_PRIME)


@pytest.mark.parametrize("offset", [0, 1])
def test_a_value_at_or_above_p_minus_one_is_refused(offset):
    with pytest.raises(ParameterRejected):
        dh.check_peer_value(SAFE_PRIME - 1 + offset, SAFE_PRIME)


def test_a_value_below_the_2_pow_1984_floor_is_refused():
    """TDLib: "g^a or g^b is not between 2^{2048-64} and dh_prime - 2^{2048-64}"."""
    with pytest.raises(ParameterRejected):
        dh.check_peer_value(LOW_BOUND - 1, SAFE_PRIME)


def test_a_value_above_the_matching_ceiling_is_refused():
    with pytest.raises(ParameterRejected):
        dh.check_peer_value(SAFE_PRIME - LOW_BOUND + 1, SAFE_PRIME)


def test_the_boundary_values_themselves_are_accepted():
    """The documented bound is inclusive: "between 2^{2048-64} and p - 2^{2048-64}"."""
    dh.check_peer_value(LOW_BOUND, SAFE_PRIME)
    dh.check_peer_value(SAFE_PRIME - LOW_BOUND, SAFE_PRIME)


def test_the_check_reads_bytes_as_a_big_endian_integer():
    """Peer values arrive as ``bytes`` on the wire. Decoding them the wrong way
    round turns a valid value into one that fails the range check, or worse."""
    value = LOW_BOUND + 12345
    assert dh.value_from_bytes(value.to_bytes(256, "big")) == value


@pytest.mark.parametrize("value", [1, LOW_BOUND - 1, SAFE_PRIME // 2])
def test_checking_a_peer_value_never_exponentiates(value, monkeypatch):
    """Refused or accepted, this check is comparisons only.

    That is the whole guarantee of the file: the range test runs to completion
    without ``pow(g_b, a, p)`` ever being reached, so a value chosen to force a weak
    shared key is rejected before the shared key exists. Asserted on the accepting
    path too - a check that only skips the work when it refuses would still let the
    normal path derive first and validate afterwards."""
    calls = []
    real_pow = builtins.pow
    monkeypatch.setattr(builtins, "pow", lambda *a, **k: (calls.append(a), real_pow(*a, **k))[1])
    try:
        dh.check_peer_value(value, SAFE_PRIME)
    except ParameterRejected:
        pass
    assert not [c for c in calls if len(c) == 3]


def test_the_refusal_carries_no_peer_value():
    """Principle IV. ``g_b`` is not secret, but it is exchange material and the
    error boundary does not make exceptions by sensitivity."""
    value = LOW_BOUND - 1
    with pytest.raises(ParameterRejected) as caught:
        dh.check_peer_value(value, SAFE_PRIME, chat_id=9)
    assert str(value) not in str(caught.value)
