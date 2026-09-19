"""protocol-reference.md §3.4 - the parity that stops a message being mirrored back.

"If any of the received in_seq_no or out_seq_no are not consistent in terms of parity
(see table above), the client is required to **immediately abort the secret chat**."
Bold in the reference, and the reason is in the sentence before it: the least
significant bit is what makes an incoming message distinguishable from one of ours
reflected back at us.

§8.3 measured this check as **absent** from the archived package, under a literal
``# TODO add checks``.
"""

import pytest

from telethon_secret_chat import actions, sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

KEY = bytes((i * 5 + 3) % 256 for i in range(256))


def a_chat(is_outbound=True):
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=is_outbound)
    chat.adopt_key(KEY)
    return chat


def wrapper(in_seq_no, out_seq_no, layer=144):
    return tl.DecryptedMessageLayer(
        random_bytes=b"\x00" * 31,
        layer=layer,
        in_seq_no=in_seq_no,
        out_seq_no=out_seq_no,
        message=tl.DecryptedMessage(random_id=1, ttl=0, message="x"),
    )


def deliver(chat, wrap, storage=None):
    return sequence.accept(chat, wrap, storage or MemoryStorage())


# --- the parity the peer must use ---------------------------------------------


def test_the_expected_parity_when_we_originated():
    """§1.6: we are A. The peer is the recipient, so ITS in_seq_no carries x=1 and
    its out_seq_no carries x=0 - the mirror of what we send."""
    chat = a_chat(is_outbound=True)
    deliver(chat, wrapper(in_seq_no=1, out_seq_no=0))  # odd in, even out
    assert chat.in_seq_no == 1


def test_the_expected_parity_when_the_peer_originated():
    chat = a_chat(is_outbound=False)
    deliver(chat, wrapper(in_seq_no=0, out_seq_no=1))  # even in, odd out
    assert chat.in_seq_no == 1


@pytest.mark.parametrize(
    "is_outbound, in_seq_no, out_seq_no",
    [
        (True, 0, 0),  # in should be odd
        (True, 1, 1),  # out should be even
        (True, 0, 1),  # both wrong
        (False, 1, 1),  # in should be even
        (False, 0, 0),  # out should be odd
    ],
)
def test_a_parity_violation_is_refused(is_outbound, in_seq_no, out_seq_no):
    chat = a_chat(is_outbound=is_outbound)
    with pytest.raises(MessageRejected):
        deliver(chat, wrapper(in_seq_no=in_seq_no, out_seq_no=out_seq_no))


@pytest.mark.parametrize("is_outbound, in_seq_no, out_seq_no", [(True, 0, 0), (False, 1, 1)])
def test_a_parity_violation_ends_the_chat(is_outbound, in_seq_no, out_seq_no):
    """ "the client is required to immediately abort the secret chat". Not a dropped
    message - the chat."""
    chat = a_chat(is_outbound=is_outbound)
    with pytest.raises(MessageRejected):
        deliver(chat, wrapper(in_seq_no=in_seq_no, out_seq_no=out_seq_no))
    assert chat.state is ChatState.CLOSED


def test_our_own_message_reflected_back_is_refused():
    """The attack the parity exists to stop. We send with (in even, out odd); a
    reflection arrives carrying exactly those, and the expected incoming parity is
    the opposite of both."""
    from telethon_secret_chat import framing

    chat = a_chat(is_outbound=True)
    mine = wrapper(
        in_seq_no=framing.transform_in_seq_no(0, True),
        out_seq_no=framing.transform_out_seq_no(0, True),
    )
    with pytest.raises(MessageRejected):
        deliver(chat, mine)
    assert chat.state is ChatState.CLOSED


def test_the_refusal_does_not_deliver_the_message():
    chat = a_chat()
    with pytest.raises(MessageRejected):
        deliver(chat, wrapper(in_seq_no=0, out_seq_no=0))
    assert chat.in_seq_no == 0, "a refused message advanced the counter"


# --- §3.6's third condition, the peer's layer ---------------------------------


def test_a_peer_walking_its_layer_backwards_is_refused():
    """§3.6: TDLib adds ``"his_layer is not monotonic"`` with no documentation
    counterpart. The reference marks it UNVERIFIED as a documented requirement and
    says the oracle is the tie-breaker - and the reason is plain: a peer that can
    lower its announced layer can lower it below 73 and out of MTProto 2.0."""
    chat = a_chat(is_outbound=True)
    deliver(chat, wrapper(in_seq_no=1, out_seq_no=0, layer=144))
    with pytest.raises(MessageRejected):
        deliver(chat, wrapper(in_seq_no=1, out_seq_no=2, layer=101))
    assert chat.state is ChatState.CLOSED


def test_a_peer_raising_its_layer_is_accepted():
    chat = a_chat(is_outbound=True)
    deliver(chat, wrapper(in_seq_no=1, out_seq_no=0, layer=101))
    deliver(chat, wrapper(in_seq_no=1, out_seq_no=2, layer=144))
    assert chat.layer == 144


def test_a_notify_layer_does_not_raise_the_bar_the_next_wrapper_must_clear():
    """The two layers TDLib keeps apart, and this package had collapsed into one.

    ``config_state_.his_layer`` is the peer's CAPABILITY: raised by a NotifyLayer
    (``[TD:SecretChatActor.cpp:2007-2008]``) and by any wrapper announcing higher
    (``:852-853``), and it is what ``current_layer()`` clamps our outgoing layer to.
    ``seq_no_state_.his_layer`` is the layer of the last ACCEPTED WRAPPER, written
    only from ``message->his_layer()`` (``:1124-1126``), and it is the one
    ``check_seq_no`` requires to be monotonic (``:902``). A NotifyLayer never
    touches it.

    Storing both in ``chat.layer`` made a NotifyLayer raise the floor every later
    wrapper had to clear, and a real client does not keep to it: it sends
    NotifyLayer at its full capability and encodes messages at
    ``min(mine, his)`` - so its first wrappers carry 73 while it still believes we
    are at 46. TDLib expects exactly that and says so at ``:848``, where the
    below-73 rejection is disabled with ``// Android app can send such messages``.

    Measured, against the owner's own Telegram Desktop on 2026-09-20: the chat
    opened, the peer announced 143, its first message arrived at a lower wrapper
    layer and this package closed the chat -
    ``the peer announced a lower layer than it had already claimed``.
    """
    import asyncio

    chat = a_chat(is_outbound=True)
    outcome = asyncio.run(
        actions.handle(None, chat, tl.DecryptedMessageActionNotifyLayer(layer=143))
    )
    assert outcome.applied
    assert chat.layer == 143, "a NotifyLayer must still raise the capability layer"

    # The peer's first real message, encoded at the layer it believed we supported.
    deliver(chat, wrapper(in_seq_no=1, out_seq_no=0, layer=73))

    assert chat.state is not ChatState.CLOSED, (
        "a wrapper below the peer's announced CAPABILITY closed the chat - the "
        "monotonic check is reading the capability instead of the last wrapper"
    )
