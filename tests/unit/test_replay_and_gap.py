"""protocol-reference.md §3.5 and §3.6 - replay, gaps, and the counter echoed back.

§3.5, the peer's counter: "Your client must check that it has received each message
with the sequence number out_seq_no starting from 0 to some current point C. It
should then expect the next message to have the sequence number out_seq_no=C+1."

- at or below C: "the local client must drop the message (repeated message). The
  client should not check the contents of the message because the original message
  could have been deleted."
- above C+1: a gap. "Note that in_seq_no is not increased upon receipt of such a
  message; it is advanced only after all preceding gaps are filled."

§3.6, our counter coming back: non-decreasing, and "if D is the out_seq_no of last
message we sent, the received in_seq_no should not be greater than D + 1". "If
in_seq_no contradicts these criteria, the local client is required to immediately
abort the secret chat."

§8.3 measured all of it as absent.
"""

from types import SimpleNamespace

import pytest

from telethon_secret_chat import SecretChatManager, sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import FakeClient, establish

KEY = bytes((i * 5 + 3) % 256 for i in range(256))


def a_chat(sent=0):
    """We are the originator, so incoming carries (in odd, out even)."""
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.adopt_key(KEY)
    chat.out_seq_no = sent  # how many WE have sent, for §3.6's D
    return chat


def peer_message(raw_out, text="x", raw_in=0):
    """A message from the peer, with §3.4's transform already applied."""
    return tl.DecryptedMessageLayer(
        random_bytes=b"\x00" * 31,
        layer=144,
        in_seq_no=2 * raw_in + 1,  # the peer is the recipient: x = 1
        out_seq_no=2 * raw_out,  # ... and x = 0 on its out
        message=tl.DecryptedMessage(random_id=raw_out, ttl=0, message=text),
    )


def texts(result):
    return [w.message.message for w in result.ready]


# --- the ordinary case --------------------------------------------------------


def test_the_expected_message_is_delivered():
    chat, store = a_chat(), MemoryStorage()
    assert texts(sequence.accept(chat, peer_message(0, "first"), store)) == ["first"]
    assert chat.in_seq_no == 1


def test_a_run_of_messages_is_delivered_in_order():
    chat, store = a_chat(), MemoryStorage()
    got = []
    for i in range(5):
        got += texts(sequence.accept(chat, peer_message(i, f"m{i}"), store))
    assert got == [f"m{i}" for i in range(5)]
    assert chat.in_seq_no == 5


# --- replay -------------------------------------------------------------------


def test_a_repeated_message_is_dropped_and_never_delivered():
    """ "the local client must drop the message (repeated message)". Dropped, not
    refused: a replay is not grounds to end the chat."""
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(0), store)
    result = sequence.accept(chat, peer_message(0, "the same one again"), store)
    assert result.ready == []
    assert chat.in_seq_no == 1
    assert chat.state is ChatState.READY


def test_an_old_message_well_below_the_counter_is_dropped():
    chat, store = a_chat(), MemoryStorage()
    for i in range(4):
        sequence.accept(chat, peer_message(i), store)
    assert sequence.accept(chat, peer_message(1, "ancient"), store).ready == []
    assert chat.in_seq_no == 4


def test_a_replay_is_not_inspected():
    """ "The client should not check the contents of the message because the original
    message could have been deleted." So a replay carrying a malformed inner message
    is still just dropped."""
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(0), store)
    broken = peer_message(0)
    broken.message = None
    assert sequence.accept(chat, broken, store).ready == []


# --- gaps ---------------------------------------------------------------------


def test_a_message_ahead_of_a_hole_is_held_not_delivered():
    chat, store = a_chat(), MemoryStorage()
    result = sequence.accept(chat, peer_message(2, "from the future"), store)
    assert result.ready == []
    assert chat.in_seq_no == 0, "in_seq_no advanced across a gap"


def test_the_hole_closing_releases_everything_behind_it_in_order():
    """§3.7: "interpret recovered messages in seq_no order first, then drain the
    queue in seq_no order"."""
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(2, "third"), store)
    sequence.accept(chat, peer_message(1, "second"), store)
    released = sequence.accept(chat, peer_message(0, "first"), store)
    assert texts(released) == ["first", "second", "third"]
    assert chat.in_seq_no == 3


def test_a_partially_filled_hole_stays_held():
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(3, "fourth"), store)
    assert texts(sequence.accept(chat, peer_message(1, "second"), store)) == []
    assert chat.in_seq_no == 0


def test_a_gap_asks_for_exactly_the_missing_span():
    """§3.7: "you can easily get the necessary start_seq_no by adding 2 to the
    out_seq_no of the last message before the hole and the end_seq_no by subtracting
    2 from the out_seq_no of the received message". Transformed values, not raw."""
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(0), store)  # last before the hole: wire 0
    result = sequence.accept(chat, peer_message(4), store)  # arrived: wire 8
    assert result.resend == (0 + 2, 8 - 2)


def test_only_one_resend_is_requested_per_hole():
    """§3.7: "if the remote client keeps sending out of sync messages, they should be
    put into the queue without sending a new request"."""
    chat, store = a_chat(), MemoryStorage()
    assert sequence.accept(chat, peer_message(3), store).resend is not None
    assert sequence.accept(chat, peer_message(4), store).resend is None
    assert sequence.accept(chat, peer_message(5), store).resend is None


def test_a_closed_hole_allows_a_later_one_to_be_requested():
    chat, store = a_chat(), MemoryStorage()
    sequence.accept(chat, peer_message(1), store)
    assert sequence.accept(chat, peer_message(0), store).ready
    assert sequence.accept(chat, peer_message(5), store).resend is not None


# --- §3.6, our counter coming back --------------------------------------------


def test_a_non_decreasing_echo_is_accepted():
    chat, store = a_chat(sent=5), MemoryStorage()
    sequence.accept(chat, peer_message(0, raw_in=1), store)
    sequence.accept(chat, peer_message(1, raw_in=1), store)
    sequence.accept(chat, peer_message(2, raw_in=3), store)
    assert chat.in_seq_no == 3


def test_an_echo_that_goes_backwards_ends_the_chat():
    """ "in_seq_no must form a non-decreasing sequence of non-negative integer
    numbers.\" """
    chat, store = a_chat(sent=5), MemoryStorage()
    sequence.accept(chat, peer_message(0, raw_in=3), store)
    with pytest.raises(MessageRejected):
        sequence.accept(chat, peer_message(1, raw_in=1), store)
    assert chat.state is ChatState.CLOSED


def test_an_echo_past_what_we_have_sent_ends_the_chat():
    """ "if D is the out_seq_no of last message we sent, the received in_seq_no should
    not be greater than D + 1". We have sent 2, so D is 1 and 2 is the ceiling.

    This is what makes "manipulations with delayed messages impossible": a peer
    cannot claim to have seen a message we have not written yet.
    """
    chat, store = a_chat(sent=2), MemoryStorage()
    sequence.accept(chat, peer_message(0, raw_in=2), store)  # exactly D+1, allowed
    with pytest.raises(MessageRejected):
        sequence.accept(chat, peer_message(1, raw_in=3), store)
    assert chat.state is ChatState.CLOSED


def test_the_echo_is_checked_before_the_message_is_queued():
    """A message held for a gap is a message whose §3.6 fields were already checked;
    holding one that violates them would defer the abort until the hole closed."""
    chat, store = a_chat(sent=1), MemoryStorage()
    with pytest.raises(MessageRejected):
        sequence.accept(chat, peer_message(4, raw_in=9), store)
    assert chat.state is ChatState.CLOSED
    assert store.take_in(chat.id) == []


# --- deep-debug 2026-09-26: a resend request that never left ------------------


class _FlakyClient(FakeClient):
    refuse_sends = False

    async def __call__(self, request):
        if self.refuse_sends and "SendEncrypted" in type(request).__name__:
            raise OSError("synthetic network failure")
        return await super().__call__(request)


async def test_a_resend_request_lost_before_it_was_queued_is_asked_again():
    """DD-04. §3.7 sends ONE request per hole, so the hole was marked requested in
    the same commit that queued the gap message - before the request itself was
    written. A failure in between (here: retrying an older unsent message) lost the
    request while the mark stayed, so every later message just joined the queue and
    the hole was never asked for again."""
    wire = SimpleNamespace(a=_FlakyClient(user_id=1000), b=FakeClient(user_id=2000))
    wire.a.peer, wire.b.peer = wire.b, wire.a
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat_a, chat_b = await establish(a, b, wire)
        got = []
        a.on("MessageReceived", lambda event: got.append(event.text))

        wire.a.refuse_sends = True
        with pytest.raises(OSError):
            await a.send_message(chat_a.id, "left pending by a failed send")
        wire.b.hold = True
        await b.send_message(chat_b.id, "m0")
        wire.b.held.clear()  # lost in transit: this is the hole
        wire.b.hold = False
        await b.send_message(chat_b.id, "m1")  # A sees the hole; its request fails

        wire.a.refuse_sends = False
        await b.send_message(chat_b.id, "m2")

        assert got == ["m0", "m1", "m2"], "the hole was never requested again"
    finally:
        await a.stop()
        await b.stop()


async def test_a_retried_resend_request_skips_what_arrived_in_the_meantime():
    """DD-09, found by review of DD-04. The span is decided when the hole opens. If
    part of the hole arrives on its own before a retry succeeds, and this side's
    echo has meanwhile told the peer to forget it, asking for the old span again is
    unsatisfiable and the peer ends the chat. The retry asks only for what is still
    missing."""
    wire = SimpleNamespace(a=_FlakyClient(user_id=1000), b=FakeClient(user_id=2000))
    wire.a.peer, wire.b.peer = wire.b, wire.a
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat_a, chat_b = await establish(a, b, wire)
        got = []
        a.on("MessageReceived", lambda event: got.append(event.text))

        wire.a.refuse_sends = True
        with pytest.raises(OSError):
            await a.send_message(chat_a.id, "left pending by a failed send")
        wire.b.hold = True
        await b.send_message(chat_b.id, "m0")
        await b.send_message(chat_b.id, "m1")
        late_m0, _lost_m1 = wire.b.held
        wire.b.held, wire.b.hold = [], False
        await b.send_message(chat_b.id, "m2")  # the hole is m0..m1; its request fails
        await wire.a.deliver(*late_m0)  # m0 turns up by itself; the retry fails too

        wire.a.refuse_sends = False
        await a.send_message(chat_a.id, "x")  # its echo lets the peer forget m0
        await b.send_message(chat_b.id, "m3")

        assert got == ["m0", "m1", "m2", "m3"]
        assert b.status(chat_b.id).state is ChatState.READY, "the peer ended the chat"
    finally:
        await a.stop()
        await b.stop()


async def test_an_unsatisfiable_resend_request_ends_the_chat():
    """DD-10, pre-existing, found by the same review. ``sequence`` closes the chat
    and raises ``ResendUnsatisfiable``; the manager then closed it again with
    ``failure.reason``, which that error did not have. The AttributeError left the
    chat open on the peer's side and reported only a generic failure."""
    wire = SimpleNamespace(a=FakeClient(user_id=1000), b=FakeClient(user_id=2000))
    wire.a.peer, wire.b.peer = wire.b, wire.a
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    try:
        chat_a, chat_b = await establish(a, b, wire)
        closed = []
        b.on("ChatClosed", closed.append)
        # B is the recipient, so its own out_seq_no is even (§3.4). It never sent 200.
        await a._send(
            a.status(chat_a.id),
            tl.DecryptedMessageService(
                random_id=1,
                action=tl.DecryptedMessageActionResend(start_seq_no=200, end_seq_no=200),
            ),
        )
        assert b.status(chat_b.id).state is ChatState.CLOSED
        assert [event.reason for event in closed] == ["a resend request could not be satisfied"]
    finally:
        await a.stop()
        await b.stop()
