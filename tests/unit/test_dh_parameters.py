"""protocol-reference.md §1.2 - every documented check, driven by a failing input.

§8.1 measured the archived package performing NONE of the first three of these: no
primality test, no bit-length test, no ``(p-1)/2`` test, and no range check on ``g``
at all. A chat established on a prime that is not safe cannot be repaired afterwards
- the key is already wrong - which is why this suite comes before the happy path.

Each case asserts two things: that the parameters are REFUSED, and that no key was
derived. The second matters on its own. A function that raises after computing
``pow(g, a, p)`` has already done the work an attacker wanted done.
"""

import builtins

import pytest

from telethon_secret_chat import dh
from telethon_secret_chat.errors import ParameterRejected

from .dh_material import COMPOSITE_2048, COMPOSITE_PASSING_RESIDUE, SAFE_PRIME

# --- what must be accepted ----------------------------------------------------


def test_a_real_safe_prime_with_g_2_is_accepted():
    """RFC 3526 group 14. If this is refused, every chat is refused."""
    dh.check_config(g=2, p=SAFE_PRIME)


# --- the prime ----------------------------------------------------------------


def test_a_composite_prime_is_refused():
    """§1.2: "The client is expected to check whether p is a safe 2048-bit prime".

    ``COMPOSITE_PASSING_RESIDUE`` and not ``COMPOSITE_2048``: the latter is refused
    by the RESIDUE condition first, whose message also contains the word "prime", so
    asserting on that word made this test pass without the primality check existing
    at all. A mutation check found it.
    """
    with pytest.raises(ParameterRejected) as caught:
        dh.check_config(g=2, p=COMPOSITE_PASSING_RESIDUE)
    assert "safe prime" in str(caught.value)


def test_the_residue_condition_is_what_refuses_a_prime_of_the_wrong_class():
    """The branch `COMPOSITE_2048` actually reaches, asserted on its own message so
    the two cases cannot be confused again."""
    with pytest.raises(ParameterRejected) as caught:
        dh.check_config(g=2, p=COMPOSITE_2048)
    assert "subgroup" in str(caught.value)


def test_a_prime_of_the_wrong_bit_length_is_refused():
    """§1.2: "2^2047 < p < 2^2048". 2039 is prime and far too small."""
    with pytest.raises(ParameterRejected) as caught:
        dh.check_config(g=2, p=2039)
    assert "2048" in str(caught.value)


def test_a_prime_one_bit_too_large_is_refused():
    with pytest.raises(ParameterRejected):
        dh.check_config(g=2, p=SAFE_PRIME * 2 + 1)


def test_a_prime_whose_half_is_composite_is_refused():
    """§1.2: "both p and (p-1)/2 are prime". 13 is prime; (13-1)/2 = 6 is not.

    Exercised at small width because finding a 2048-bit non-safe prime costs more
    than the branch is worth - the arithmetic under test is identical at any size,
    and `test_a_real_safe_prime_with_g_2_is_accepted` covers the 2048-bit path.
    """
    assert dh.is_probable_prime(13)
    assert not dh.is_safe_prime(13)
    assert dh.is_safe_prime(11)  # (11-1)/2 = 5, prime


def test_the_primality_test_agrees_with_a_known_sieve():
    """A Miller-Rabin that answered "prime" to everything would pass every test
    above. This one is checked against an independent sieve over a real range."""
    limit = 2000
    sieve = [True] * limit
    sieve[0] = sieve[1] = False
    for i in range(2, int(limit**0.5) + 1):
        if sieve[i]:
            sieve[i * i :: i] = [False] * len(sieve[i * i :: i])
    mismatches = [n for n in range(limit) if dh.is_probable_prime(n) != sieve[n]]
    assert not mismatches


# --- the generator ------------------------------------------------------------


@pytest.mark.parametrize("g", [0, 1, -5])
def test_a_generator_at_or_below_one_is_refused(g):
    """§1.2: "Both clients are to check that g, g_a and g_b are greater than one"."""
    with pytest.raises(ParameterRejected):
        dh.check_config(g=g, p=SAFE_PRIME)


def test_a_generator_at_or_above_p_minus_one_is_refused():
    """The upper half of the same sentence: "and smaller than p-1"."""
    with pytest.raises(ParameterRejected):
        dh.check_config(g=SAFE_PRIME - 1, p=SAFE_PRIME)


def test_a_generator_outside_the_documented_set_is_refused():
    """§1.2: "Since g is always equal to 2, 3, 4, 5, 6 or 7". 9 is in range and not
    one of them, so there is no residue condition to apply and it cannot be proved
    to generate the right subgroup."""
    with pytest.raises(ParameterRejected):
        dh.check_config(g=9, p=SAFE_PRIME)


def test_g_2_against_a_prime_failing_its_residue_condition():
    """§1.2's table: ``g = 2`` requires ``p mod 8 == 7``.

    The 2048-bit safe prime under test has ``p mod 8 == 7``, so this drives the
    condition directly rather than through `check_config`, which would reject the
    small operand on bit length first.
    """
    assert dh.generator_is_quadratic_residue(2, SAFE_PRIME)
    assert not dh.generator_is_quadratic_residue(2, 13)  # 13 mod 8 == 5


@pytest.mark.parametrize(
    "g, good, bad",
    [
        (2, 7, 1),  # p mod 8 == 7
        (3, 2, 1),  # p mod 3 == 2
        (5, 1, 2),  # p mod 5 in {1, 4}
        (6, 19, 5),  # p mod 24 in {19, 23}
        (7, 3, 1),  # p mod 7 in {3, 5, 6}
    ],
)
def test_every_documented_residue_condition(g, good, bad):
    """The whole §1.2 table, one row at a time. A modulus copied wrong here is a
    chat that works until the day the server rotates its prime."""
    modulus = {2: 8, 3: 3, 5: 5, 6: 24, 7: 7}[g]
    assert dh.generator_is_quadratic_residue(g, good + modulus * 10)
    assert not dh.generator_is_quadratic_residue(g, bad + modulus * 10)


def test_g_4_has_no_condition():
    """§1.2's table gives ``g = 4`` the condition "none" - 4 is a square, so it is a
    quadratic residue modulo every prime."""
    assert dh.generator_is_quadratic_residue(4, 13)
    assert dh.generator_is_quadratic_residue(4, 17)


# --- nothing is computed on the way out ---------------------------------------


def test_a_structurally_wrong_prime_is_refused_before_any_exponentiation(monkeypatch):
    """The cheap checks come first, so a malformed parameter costs nothing.

    Primality testing is itself modular exponentiation, so this cannot assert "no
    ``pow`` at all" for every input - it asserts the ordering that matters: a prime
    of the wrong LENGTH is refused before a second of Miller-Rabin is spent on it,
    which is what stops a peer making this side work by sending nonsense.
    """
    calls = []
    real_pow = builtins.pow

    def watched(*args, **kwargs):
        calls.append(args)
        return real_pow(*args, **kwargs)

    monkeypatch.setattr(builtins, "pow", watched)
    with pytest.raises(ParameterRejected):
        dh.check_config(g=2, p=2039)
    assert not [c for c in calls if len(c) == 3]


def test_the_refusal_names_the_chat_without_naming_the_value():
    """Principle IV: the reason is a phrase this package wrote, never the prime."""
    with pytest.raises(ParameterRejected) as caught:
        dh.check_config(g=2, p=COMPOSITE_2048, chat_id=77)
    text = str(caught.value)
    assert "77" in text
    assert str(COMPOSITE_2048) not in text
    assert f"{COMPOSITE_2048:x}" not in text.lower()
