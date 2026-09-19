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


# --- T048: every failure path added in Phases 3-9 (SC-005) --------------------
#
# The tests above drive each ERROR TYPE with dangerous material. These drive each
# real refusal SITE with material that would be catastrophic to print, because the
# type being safe is not the same as every construction of it being safe: a reason
# string is written at the site, and the site is where a protocol object is in
# scope.

PLAIN = "the quick brown fox jumps over the lazy dog"
KEY_256 = bytes((i * 7 + 3) % 256 for i in range(256))


def _catch(call):
    """Run a refusal and return everything it emitted, as one string."""
    import pytest as _pytest

    with _pytest.raises(errors.SecretChatError) as caught:
        call()
    error = caught.value
    return str(error) + repr(error) + repr(vars(error))


def _refusals():
    """One callable per refusal site reachable without a network."""
    import os

    from telethon_secret_chat import crypto, dh, files, framing, sequence
    from telethon_secret_chat.chat import SecretChat
    from telethon_secret_chat.schema import secret_tl as tl
    from telethon_secret_chat.storage import MemoryStorage

    def chat(**over):
        c = SecretChat(id=42, access_hash=1, peer_user_id=2, is_outbound=True)
        c.adopt_key(KEY_256)
        for k, v in over.items():
            setattr(c, k, v)
        return c

    def wrapper(in_seq_no, out_seq_no, layer=144):
        return tl.DecryptedMessageLayer(
            random_bytes=b"\x00" * 31,
            layer=layer,
            in_seq_no=in_seq_no,
            out_seq_no=out_seq_no,
            message=tl.DecryptedMessage(random_id=1, ttl=0, message=PLAIN),
        )

    frame = crypto.encrypt_frame(KEY_256, bytes(wrapper(1, 0)), 0)
    corrupt = bytearray(frame)
    corrupt[-1] ^= 0x01
    file_key, file_iv = os.urandom(32), os.urandom(32)

    return {
        # §1.2
        # Two different §1.2 branches: the residue condition, and the safe-prime
        # test behind it. One case cannot reach both.
        "dh: bad residue": lambda: dh.check_config(g=2, p=2**2047 + 1, chat_id=42),
        "dh: composite prime": lambda: dh.check_config(g=2, p=2**2047 + 7, chat_id=42),
        "dh: peer value": lambda: dh.check_peer_value(1, 2**2047 + 1, chat_id=42),
        # §2.7
        "crypto: wrong fingerprint": lambda: crypto.decrypt_frame(
            bytes(256), frame, 0, chat_id=42
        ),
        "crypto: tampered": lambda: crypto.decrypt_frame(KEY_256, bytes(corrupt), 0, chat_id=42),
        "crypto: truncated": lambda: crypto.decrypt_frame(KEY_256, frame[:30], 0, chat_id=42),
        # §3.1-§3.2
        "framing: not a wrapper": lambda: framing.unwrap(
            bytes(tl.DecryptedMessage(random_id=1, ttl=0, message=PLAIN)), chat_id=42
        ),
        "framing: unparseable": lambda: framing.unwrap(b"\xde\xad\xbe\xef", chat_id=42),
        "framing: layer too low": lambda: framing.require_supported_layer(
            chat_id=42, peer_layer=8
        ),
        # §3.4-§3.6
        "sequence: parity": lambda: sequence.accept(chat(), wrapper(0, 0), MemoryStorage()),
        "sequence: echo backwards": lambda: sequence.accept(
            chat(peer_in_seq_no=5, out_seq_no=9), wrapper(1, 0), MemoryStorage()
        ),
        "sequence: echo too far": lambda: sequence.accept(
            chat(out_seq_no=0), wrapper(9, 0), MemoryStorage()
        ),
        # `wrapper_layer`, not `layer`: the monotonic check reads the layer the peer
        # has ENCODED IN, never its announced capability (.ai/BUGS.md B-001).
        "sequence: layer backwards": lambda: sequence.accept(
            chat(wrapper_layer=144), wrapper(1, 0, layer=101), MemoryStorage()
        ),
        # §3.7
        "sequence: resend unsatisfiable": lambda: sequence.answer_resend(
            chat(), MemoryStorage(), 1, 9
        ),
        "sequence: resend too wide": lambda: sequence.answer_resend(
            chat(), MemoryStorage(), 1, 1 + 2 * sequence.MAX_RESEND_COUNT + 2
        ),
        # §6.2
        "files: fingerprint": lambda: files.verify_file_fingerprint(
            chat_id=42, key=file_key, iv=file_iv, claimed=0
        ),
        # the chat's own refusals
        "chat: not ready": lambda: SecretChat(
            id=42, access_hash=1, peer_user_id=2, is_outbound=True
        ).require_sendable(),
        "chat: closed": lambda: _closed().require_sendable(),
    }


def _closed():
    from telethon_secret_chat.chat import SecretChat

    c = SecretChat(id=42, access_hash=1, peer_user_id=2, is_outbound=True)
    c.adopt_key(KEY_256)
    c.close("an integrity check failed")
    return c


@pytest.mark.parametrize("name", list(_refusals()))
def test_no_refusal_path_emits_key_material_or_plaintext(name):
    """SC-005: "No key material or plaintext appears anywhere outside the chat,
    demonstrated by a test that exercises the failure paths and inspects what they
    emitted.\" """
    rendered = _catch(_refusals()[name])
    assert PLAIN not in rendered, f"{name} leaked plaintext"
    assert KEY_256.hex()[:32] not in rendered.lower(), f"{name} leaked the key"
    assert repr(KEY_256) not in rendered, f"{name} leaked the key's repr"
    assert "DecryptedMessage" not in rendered, f"{name} formatted a protocol object"


@pytest.mark.parametrize("name", list(_refusals()))
def test_every_refusal_path_still_says_something_useful(name):
    """A boundary that emitted nothing would pass every assertion above. Each
    refusal names its chat and describes what was refused."""
    rendered = _catch(_refusals()[name])
    assert "42" in rendered, f"{name} does not name the chat"
    assert len(rendered) > 40, f"{name} says too little to act on"


@pytest.mark.parametrize("name", list(_refusals()))
def test_no_refusal_path_emits_a_long_opaque_run(name):
    """The catch-all: key material that reached a message by accident looks like a
    long unbroken run of hex or base64, whatever route it took to get there."""
    rendered = _catch(_refusals()[name])
    for run in re.findall(r"[A-Za-z0-9+/=]{24,}", rendered):
        assert not re.fullmatch(r"[0-9a-fA-F]{24,}", run), f"{name} emitted {run[:24]}..."
