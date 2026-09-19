"""protocol-reference.md §3.7 - answering a peer's resend, and the bound on it.

Four rules, each with a failure mode of its own:

- "decryptedMessageActionResend must always be interpreted immediately upon receipt
  in all cases, even if its out_seq_no>=C+1" - the one exception to in-order
  interpretation, because a resend request stuck behind the gap it is trying to
  close would never be read.
- "each decryptedMessageActionResend must only be handled once" - TDLib rewrites the
  action to Noop in place so a binlog replay cannot resend twice.
- ``MAX_RESEND_COUNT = 1000``, and TDLib rejects a wider request with "Can't resend
  too many messages".
- "If a local client receives decryptedMessageActionResend but is unable to satisfy
  the request, it must abort the secret chat."

§8.3 measured the archived package answering a resend by sending a NEW message with
a NEW out_seq_no - which cannot fill the peer's hole - from a retention dict that
was explicitly never persisted, and with no bound at all.
"""

import pytest

from telethon_secret_chat import sequence
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import ResendUnsatisfiable
from telethon_secret_chat.storage import MemoryStorage

KEY = bytes((i * 5 + 3) % 256 for i in range(256))


def a_chat():
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.adopt_key(KEY)
    return chat


def retained(store, chat, count):
    """``count`` messages this side sent, as the retention queue holds them. We are
    the originator, so our out_seq_no carries x = 1."""
    for raw in range(count):
        store.queue_out(chat.id, {"seq_no": 2 * raw + 1, "body": f"body-{raw}"})


# --- answering ----------------------------------------------------------------


def test_the_requested_span_comes_back_in_order():
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 6)
    answer = sequence.answer_resend(chat, store, start_seq_no=3, end_seq_no=7)
    assert [m["body"] for m in answer] == ["body-1", "body-2", "body-3"]


def test_the_span_bounds_are_inclusive():
    """§3.7 computes them as "adding 2 to the out_seq_no of the last message before
    the hole" and "subtracting 2 from the out_seq_no of the received message" - both
    ends name a message that must come back."""
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 4)
    assert [m["body"] for m in sequence.answer_resend(chat, store, 1, 1)] == ["body-0"]
    assert len(sequence.answer_resend(chat, store, 1, 7)) == 4


def test_a_single_message_span_is_satisfied():
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 3)
    assert [m["body"] for m in sequence.answer_resend(chat, store, 5, 5)] == ["body-2"]


def test_resending_does_not_consume_the_retention():
    """The peer may ask twice - the request can itself be lost. What must happen
    exactly once is HANDLING a given request, which is the manager's dedup, not the
    queue's."""
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 3)
    assert len(sequence.answer_resend(chat, store, 1, 5)) == 3
    assert len(sequence.answer_resend(chat, store, 1, 5)) == 3


# --- what cannot be satisfied -------------------------------------------------


def test_a_span_below_what_is_retained_ends_the_chat():
    """ "If a local client receives decryptedMessageActionResend but is unable to
    satisfy the request, it must abort the secret chat.\" """
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 3)
    store.drop_out(chat.id, 1)  # the peer acknowledged the first, so it is gone
    with pytest.raises(ResendUnsatisfiable):
        sequence.answer_resend(chat, store, 1, 5)
    assert chat.state is ChatState.CLOSED


def test_a_span_beyond_what_was_ever_sent_ends_the_chat():
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 2)
    with pytest.raises(ResendUnsatisfiable):
        sequence.answer_resend(chat, store, 1, 99)
    assert chat.state is ChatState.CLOSED


def test_a_span_wider_than_the_cap_is_refused():
    """§3.7: ``MAX_RESEND_COUNT = 1000``. The spec's Assumptions: "adopt the
    reference implementation's span cap rather than inventing a number"."""
    chat, store = a_chat(), MemoryStorage()
    assert sequence.MAX_RESEND_COUNT == 1000
    with pytest.raises(ResendUnsatisfiable):
        sequence.answer_resend(chat, store, 1, 1 + 2 * sequence.MAX_RESEND_COUNT + 2)
    assert chat.state is ChatState.CLOSED


def test_a_span_at_exactly_the_cap_is_not_refused_for_width():
    """The cap is a limit, not an off-by-one. This one fails on retention instead,
    which is a different reason - what matters is that the width alone did not
    reject it."""
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, sequence.MAX_RESEND_COUNT)
    answer = sequence.answer_resend(chat, store, 1, 2 * (sequence.MAX_RESEND_COUNT - 1) + 1)
    assert len(answer) == sequence.MAX_RESEND_COUNT


def test_an_inverted_span_is_refused():
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 5)
    with pytest.raises(ResendUnsatisfiable):
        sequence.answer_resend(chat, store, 7, 3)
    assert chat.state is ChatState.CLOSED


def test_the_refusal_names_the_span_and_nothing_else():
    """Principle IV. §3.7's spans are positions, not content - safe to state - but
    the retained BODIES are plaintext and must not appear."""
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 2)
    store.queue_out(chat.id, {"seq_no": 99, "body": "a sentence from a retained message"})
    with pytest.raises(ResendUnsatisfiable) as caught:
        sequence.answer_resend(chat, store, 1, 4001)
    assert "a sentence from a retained message" not in str(caught.value)


# --- retention (§3.7, §3.8) ---------------------------------------------------


def test_acknowledged_messages_stop_being_retained():
    """§3.6's echo is what says the peer processed them; anything at or below it can
    no longer be asked for."""
    chat, store = a_chat(), MemoryStorage()
    retained(store, chat, 5)
    sequence.forget_acknowledged(chat, store, peer_in_seq_no_raw=2)
    assert [m["body"] for m in store.retained_out(chat.id)] == ["body-2", "body-3", "body-4"]


def test_retention_survives_a_new_backend_instance(tmp_path):
    """§8.3: the archived package's retention was "explicitly not persisted", so it
    was empty after any restart and every resend request was unsatisfiable - which
    the protocol answers by ending the chat."""
    from telethon_secret_chat.storage import FileStorage

    chat = a_chat()
    store = FileStorage(tmp_path / "chats.db")
    retained(store, chat, 3)
    revived = FileStorage(tmp_path / "chats.db")
    assert len(sequence.answer_resend(chat, revived, 1, 5)) == 3


# --- handled immediately, and exactly once (§3.7) -----------------------------


async def test_a_resend_request_is_answered_even_while_it_is_out_of_order():
    """§3.7's one exception to in-order interpretation: "decryptedMessageActionResend
    must always be interpreted immediately upon receipt in all cases, even if its
    out_seq_no>=C+1".

    The reason is circular otherwise: a resend request that arrives after the hole
    it is trying to close would be queued BEHIND that hole and never read, so the
    hole never closes.
    """
    from telethon.tl import functions

    from .fake_client import Wire, establish
    from telethon_secret_chat import SecretChatManager
    from telethon_secret_chat.schema import secret_tl as stl

    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    chat_a, chat_b = await establish(a, b, wire)

    await a.send_message(chat_a.id, "the one that will be asked for")
    before = len(wire.a.sent)

    # B asks for that message, in a service message carrying an out_seq_no far ahead
    # of where A is - so it would be QUEUED if the exception did not exist.
    await b._send(
        b.status(chat_b.id),
        stl.DecryptedMessageService(
            random_id=1,
            # A has sent two messages since the chat opened: its NotifyLayer
            # (§7.3, raw 0) and the text above (raw 1). A is the originator, so its
            # out_seq_no carries x=1 and the text is wire 3.
            action=stl.DecryptedMessageActionResend(start_seq_no=3, end_seq_no=3),
        ),
    )
    resent = [
        r for r in wire.a.sent[before:] if isinstance(r, functions.messages.SendEncryptedRequest)
    ]
    assert resent, "the resend request was never answered"
    await a.stop()
    await b.stop()
