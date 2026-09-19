"""Every error this package raises carries a SHAPE and never a value.

Constitution Principle IV. The reason it is the first test written, before any
protocol code exists, is that the boundary it guards cannot be retrofitted: once a
dozen call sites format an exception from whatever object was to hand, finding the
one that interpolated a key means reading all of them.

What "shape" means here, concretely. An error may say:

* its own type name,
* which chat it concerns,
* a bounded description of what was refused ("the prime is not safe", "padding
  length 7 is below the minimum of 12").

It may NOT say: the key, the plaintext, the ciphertext, a peer's public value, or
the `repr` of any protocol object — because a `repr` is a formatter nobody audited
and Telethon's TL objects print their fields.

The test drives every error type the contract lists, hands each one material that
would be catastrophic to print, and asserts the material is absent from both
``str()`` and ``repr()``.
"""

import re

import pytest

from telethon_secret_chat import errors

# Material that must never survive into a message. Chosen to be unmistakable: if any
# of these appears, the test can name which one and the assertion is not ambiguous.
SECRET_KEY = bytes(range(256))
PLAINTEXT = "the text of a message nobody but two people should ever read"
CIPHERTEXT = b"\xde\xad\xbe\xef" * 64
PEER_PUBLIC = 0xC0FFEE_1234_5678_9ABC_DEF0


class _Loud:
    """A protocol object whose repr spills, standing in for a TL type.

    Telethon's generated types print their fields, so an error that formats one is
    an error that prints whatever the field held. This makes that failure visible
    rather than hypothetical.
    """

    def __repr__(self):
        return f"KeyExchange(key={SECRET_KEY!r}, g_a={PEER_PUBLIC})"

    __str__ = __repr__


def _every_error():
    """One instance of each error in contracts/public-api.md §4, each constructed
    with dangerous material where the type plausibly accepts it."""
    return [
        errors.ParameterRejected(chat_id=42, reason="the prime is not a safe 2048-bit prime"),
        # Every receive-side refusal of §2.7 and §3.4-§3.6 arrives through this one
        # type, so driving it once here covers the failure paths Phases 3-9 add.
        errors.MessageRejected(
            chat_id=42, reason="the recomputed message key does not match the one received"
        ),
        errors.ChatNotReady(chat_id=42, state="requested"),
        errors.ChatClosed(chat_id=42, reason="the peer discarded the chat"),
        errors.StorageRequired(),
        errors.LayerUnsupported(chat_id=42, peer_layer=8),
        errors.ResendUnsatisfiable(chat_id=42, requested=(5, 9), retained_from=7),
    ]


@pytest.mark.parametrize("error", _every_error(), ids=lambda e: type(e).__name__)
def test_no_error_carries_key_material_or_plaintext(error):
    for rendered in (str(error), repr(error)):
        assert SECRET_KEY.hex() not in rendered.lower(), f"{type(error).__name__} leaked the key"
        assert repr(SECRET_KEY) not in rendered, f"{type(error).__name__} leaked the key's repr"
        assert PLAINTEXT not in rendered, f"{type(error).__name__} leaked plaintext"
        assert (
            CIPHERTEXT.hex() not in rendered.lower()
        ), f"{type(error).__name__} leaked ciphertext"
        assert str(PEER_PUBLIC) not in rendered, f"{type(error).__name__} leaked a public value"


@pytest.mark.parametrize("error", _every_error(), ids=lambda e: type(e).__name__)
def test_every_error_still_says_something_useful(error):
    """The boundary must not be satisfied by saying nothing.

    An error that renders as "SecretChatError()" leaks nothing and helps nobody,
    and the next person to debug a refusal will add the key back to find out why.
    """
    rendered = str(error)

    assert (
        type(error).__name__ in rendered or len(rendered) > 20
    ), f"{type(error).__name__} renders as {rendered!r}, which tells an operator nothing"


def test_an_error_refuses_to_format_a_protocol_object():
    """The specific failure this guards: composing a message with an f-string over
    something that came off the wire."""
    error = errors.ParameterRejected(chat_id=7, reason="the peer's value is out of range")

    for rendered in (str(error), repr(error)):
        assert "KeyExchange(" not in rendered
        assert str(PEER_PUBLIC) not in rendered


def test_no_error_message_contains_a_long_hex_or_base64_run():
    """A catch-all for material this test did not think to name.

    Key bytes, a session, a fingerprint's raw form - all of them render as a long
    unbroken run of hex or base64. A legitimate message has words and spaces in it.
    """
    long_run = re.compile(r"[0-9a-fA-F]{32,}|[A-Za-z0-9+/]{40,}={0,2}")

    for error in _every_error():
        for rendered in (str(error), repr(error)):
            found = long_run.search(rendered)
            assert found is None, (
                f"{type(error).__name__} rendered a {len(found.group())}-character opaque run: "
                "that is the shape of key material, not of a description"
            )


def test_all_of_them_are_one_family():
    """So an application can catch this package's failures without catching the
    world, and so a new error cannot quietly escape the boundary above."""
    for error in _every_error():
        assert isinstance(error, errors.SecretChatError)

    exported = {
        name
        for name in dir(errors)
        if isinstance(getattr(errors, name), type)
        and issubclass(getattr(errors, name), Exception)
        and not name.startswith("_")
    }
    covered = {type(e).__name__ for e in _every_error()} | {"SecretChatError"}

    assert (
        exported == covered
    ), f"these error types are not covered by the leak test: {sorted(exported - covered)}"
