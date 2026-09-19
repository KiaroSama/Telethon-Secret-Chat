"""protocol-reference.md §3.1-§3.3 and §7 - what wraps a message, and at which layer.

§3.1: every message, service messages included, travels inside
``decryptedMessageLayer``. §3.2: the layer it announces starts at 46. §3.3:
``random_bytes`` - the schema gives no minimum and the oracle always sends 31.

§7 is here too because the layer in the wrapper is the layer §7.4 computes:
``clamp(his_layer, 73, 144)``. §8.7 measured the archived package using the raw
remote value with no floor, and initialising that value to 101 rather than the
documented 46 - so an un-notified peer was assumed to speak a layer it had never
claimed.
"""

import pytest

from telethon_secret_chat import framing
from telethon_secret_chat.errors import LayerUnsupported, MessageRejected
from telethon_secret_chat.schema import secret_tl as tl


def a_message():
    return tl.DecryptedMessage(random_id=1234, ttl=0, message="hello")


# --- §3.1 the wrapper ---------------------------------------------------------


def test_a_message_is_wrapped_in_decrypted_message_layer():
    wrapped = framing.wrap(a_message(), layer=144, in_seq_no=4, out_seq_no=7)
    assert isinstance(wrapped, tl.DecryptedMessageLayer)
    assert wrapped.layer == 144 and wrapped.in_seq_no == 4 and wrapped.out_seq_no == 7


def test_a_service_message_is_wrapped_too():
    """§3.1: "Note that any service messages in secret chats must also increment the
    seq_no" - which means they are wrapped like anything else."""
    service = tl.DecryptedMessageService(random_id=9, action=tl.DecryptedMessageActionNoop())
    assert isinstance(
        framing.wrap(service, layer=73, in_seq_no=0, out_seq_no=0), tl.DecryptedMessageLayer
    )


def test_a_wrapped_message_survives_serialization():
    wrapped = framing.wrap(a_message(), layer=144, in_seq_no=2, out_seq_no=3)
    back = framing.unwrap(bytes(wrapped))
    assert back.message.message == "hello" and back.out_seq_no == 3


# --- §3.3 random_bytes --------------------------------------------------------


def test_random_bytes_is_thirty_one_bytes():
    """§3.3: the schema states no minimum and TDLib always sends exactly 31:
    ``BufferSlice random_bytes(31)``. The reference marks any shorter value
    UNVERIFIED and says not to derive a rule from the archived package, which used
    15, 19 or 23 - and picked between them with the ``random`` module."""
    assert len(framing.wrap(a_message(), layer=144, in_seq_no=0, out_seq_no=0).random_bytes) == 31


def test_random_bytes_differs_every_time():
    seen = {
        bytes(framing.wrap(a_message(), layer=144, in_seq_no=0, out_seq_no=0).random_bytes)
        for _ in range(30)
    }
    assert len(seen) == 30


# --- §3.2 / §7.2 the layer floor ----------------------------------------------


def test_a_wrapper_below_layer_46_is_refused():
    """§3.2: "with an indication of the supported layer (starting with 46)"."""
    body = bytes(
        tl.DecryptedMessageLayer(
            random_bytes=b"\x00" * 31, layer=45, in_seq_no=0, out_seq_no=0, message=a_message()
        )
    )
    with pytest.raises(MessageRejected):
        framing.unwrap(body)


def test_a_body_that_is_not_a_wrapper_is_refused():
    """A bare DecryptedMessage with no wrapper. §3.1 makes the wrapper mandatory, so
    accepting one without it would accept a message carrying no seq_no at all."""
    with pytest.raises(MessageRejected):
        framing.unwrap(bytes(a_message()))


def test_an_unparseable_body_is_refused_not_guessed():
    with pytest.raises(MessageRejected):
        framing.unwrap(b"\xde\xad\xbe\xef\x00\x00\x00\x00")


# --- §7.2 the remote layer ----------------------------------------------------


def test_the_remote_layer_starts_at_46():
    """§7.2: "When the secret chat is first created, this value should be
    initialized to 46." §8.7 measured the archived package defaulting to 101."""
    assert framing.INITIAL_REMOTE_LAYER == 46


def test_the_remote_layer_only_ever_rises():
    """§7.2: TDLib "raises config_state_.his_layer ... it never lowers it", and §3.6
    rejects a message whose layer is below the stored one. §8.7 measured the
    archived package assigning unconditionally, so a peer could walk its layer
    down - and a peer that can walk its layer down can walk it below 73."""
    assert framing.raise_remote_layer(46, 101) == 101
    assert framing.raise_remote_layer(101, 46) == 101
    assert framing.raise_remote_layer(101, 101) == 101


# --- §7.4 the layer we send at ------------------------------------------------


@pytest.mark.parametrize(
    "his, expected",
    [
        (46, 73),  # clamped UP to the floor: we do not speak below 73
        (73, 73),
        (101, 101),
        (144, 144),
        (223, 144),  # clamped DOWN to what we can parse
    ],
)
def test_the_outgoing_layer_is_clamped_between_73_and_144(his, expected):
    """§7.4: TDLib's ``current_layer()`` is ``clamp(his_layer, 73, 144)``. The lower
    clamp is what stops us announcing a layer whose messages we would then have to
    encrypt with MTProto 1.0; the upper is what stops us announcing constructors we
    cannot parse (§7.3)."""
    assert framing.outgoing_layer(his) == expected


def test_a_peer_that_cannot_reach_73_is_refused():
    """FR-017 and §7.5: this package implements no MTProto 1.0, so a peer that has
    announced a layer below 73 gets a stated refusal rather than a silent
    downgrade. Note the asymmetry with the clamp above: 46 is the ASSUMED starting
    value and is not a claim, while an explicit NotifyLayer below 73 is."""
    with pytest.raises(LayerUnsupported):
        framing.require_supported_layer(chat_id=3, peer_layer=72)
    framing.require_supported_layer(chat_id=3, peer_layer=73)


# --- §3.4 the seq_no transform ------------------------------------------------


@pytest.mark.parametrize(
    "is_outbound, raw_in, raw_out, expect_in, expect_out",
    [
        # §3.4's table. "chat initiated by sender" - the sender of THIS message.
        (True, 0, 0, 0, 1),  # we initiated: in even, out odd
        (True, 3, 5, 6, 11),
        (False, 0, 0, 1, 0),  # we accepted: in odd, out even
        (False, 3, 5, 7, 10),
    ],
)
def test_the_seq_no_transform(is_outbound, raw_in, raw_out, expect_in, expect_out):
    """§3.4: "transformation according to formula 2*raw_seq_no+x".

    "In this way the least significant bit of each seq_no field included in the
    message is different for incoming and outgoing messages. This is done to prevent
    a possible attacker from mirroring the messages."
    """
    assert framing.transform_in_seq_no(raw_in, is_outbound) == expect_in
    assert framing.transform_out_seq_no(raw_out, is_outbound) == expect_out


def test_the_transform_is_reversible():
    for is_outbound in (True, False):
        for raw in range(10):
            wire = framing.transform_out_seq_no(raw, is_outbound)
            assert framing.raw_seq_no(wire) == raw
