"""data-model.md §1 - the chat's state machine, and the invariant inside it.

Two things this file is really about.

**Terminal means terminal.** "§3.5/§3.6 name failures that must end the chat; a chat
that reached ``closed`` is never revived, because reviving it would mean continuing
on counters whose integrity is exactly what failed."

**The key and its fingerprint are one unit.** §1.5 derives one from the other, so a
chat holding key A with fingerprint B decrypts nothing and looks, from outside,
exactly like a peer problem. There is no setter for either alone.
"""

import pytest

from telethon_secret_chat import crypto
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.errors import ChatClosed, ChatNotReady

KEY = bytes((i * 5 + 3) % 256 for i in range(256))
OTHER_KEY = bytes((i * 9 + 1) % 256 for i in range(256))


def a_chat(state=ChatState.READY, is_outbound=True, **over):
    chat = SecretChat(
        id=101,
        access_hash=2222,
        peer_user_id=333,
        is_outbound=is_outbound,
        **over,
    )
    if state in (ChatState.READY, ChatState.REKEYING):
        chat.state = ChatState.REQUESTED if is_outbound else ChatState.PENDING
        chat.adopt_key(KEY)
        if state is ChatState.REKEYING:
            chat.transition_to(ChatState.REKEYING)
    else:
        chat.state = state
    return chat


# --- the transitions ----------------------------------------------------------


def test_an_outbound_chat_starts_requested():
    chat = SecretChat(id=1, access_hash=2, peer_user_id=3, is_outbound=True)
    assert chat.state is ChatState.REQUESTED


def test_an_inbound_chat_starts_pending():
    chat = SecretChat(id=1, access_hash=2, peer_user_id=3, is_outbound=False)
    assert chat.state is ChatState.PENDING


@pytest.mark.parametrize(
    "start, target",
    [
        (ChatState.REQUESTED, ChatState.READY),
        (ChatState.REQUESTED, ChatState.CLOSED),
        (ChatState.PENDING, ChatState.READY),
        (ChatState.PENDING, ChatState.CLOSED),
        (ChatState.READY, ChatState.REKEYING),
        (ChatState.READY, ChatState.CLOSED),
        (ChatState.REKEYING, ChatState.READY),
        (ChatState.REKEYING, ChatState.CLOSED),
    ],
)
def test_the_legal_transitions_are_allowed(start, target):
    chat = a_chat(state=start)
    chat.transition_to(target)
    assert chat.state is target


@pytest.mark.parametrize(
    "start, target",
    [
        (ChatState.REQUESTED, ChatState.PENDING),
        (ChatState.REQUESTED, ChatState.REKEYING),
        (ChatState.PENDING, ChatState.REQUESTED),
        (ChatState.READY, ChatState.REQUESTED),
        (ChatState.READY, ChatState.PENDING),
        (ChatState.REKEYING, ChatState.PENDING),
    ],
)
def test_the_illegal_transitions_are_refused(start, target):
    chat = a_chat(state=start)
    with pytest.raises(ChatClosed if start is ChatState.CLOSED else ValueError):
        chat.transition_to(target)


@pytest.mark.parametrize("target", list(ChatState))
def test_closed_is_terminal(target):
    """Every transition out of ``closed``, including back to ``closed``."""
    chat = a_chat(state=ChatState.CLOSED)
    with pytest.raises(ChatClosed):
        chat.transition_to(target)
    assert chat.state is ChatState.CLOSED


# --- what may be sent, and when -----------------------------------------------


@pytest.mark.parametrize("state", [ChatState.READY, ChatState.REKEYING])
def test_send_works_in_ready_and_in_rekeying(state):
    """FR-013 and §4.8: "Both sides keep encrypting with the old key until the commit
    point of their own role". A chat mid-exchange is still a working chat, and a
    message issued during one must be delivered rather than dropped."""
    a_chat(state=state).require_sendable()


@pytest.mark.parametrize("state", [ChatState.REQUESTED, ChatState.PENDING])
def test_send_refuses_before_the_key_exists(state):
    chat = a_chat(state=state)
    with pytest.raises(ChatNotReady) as caught:
        chat.require_sendable()
    assert state.value in str(caught.value)


def test_send_refuses_on_a_closed_chat_with_a_stated_reason():
    """The spec's edge cases: "sending refuses rather than throwing an unrelated
    transport error"."""
    chat = a_chat(state=ChatState.READY)
    chat.close("the peer discarded the chat")
    with pytest.raises(ChatClosed) as caught:
        chat.require_sendable()
    assert "the peer discarded the chat" in str(caught.value)


# --- the key and the fingerprint ----------------------------------------------


def test_adopting_a_key_sets_its_fingerprint_in_the_same_call():
    chat = SecretChat(id=1, access_hash=2, peer_user_id=3, is_outbound=True)
    chat.adopt_key(KEY)
    assert chat.key == KEY
    assert chat.key_fingerprint == crypto.key_fingerprint(KEY)
    assert chat.state is ChatState.READY


def test_a_key_of_the_wrong_length_is_refused():
    """§1.4 pads every shared key to exactly 256 bytes. A shorter one shifts every
    §2.4 substring."""
    chat = SecretChat(id=1, access_hash=2, peer_user_id=3, is_outbound=True)
    with pytest.raises(ValueError):
        chat.adopt_key(KEY[:255])


def test_the_key_and_fingerprint_cannot_drift_apart():
    """data-model.md §1: "They are one atomic unit." Replacing the key replaces the
    fingerprint; there is no path that sets one without the other."""
    chat = a_chat()
    chat.adopt_key(OTHER_KEY)
    assert chat.key_fingerprint == crypto.key_fingerprint(OTHER_KEY)


# --- the per-side asymmetry ---------------------------------------------------


def test_the_crypto_x_follows_the_side():
    """§2.5: "I am the originator => my outgoing messages use x = 0 and the messages
    I receive use x = 8". §1.6 calls getting this wrong the most likely silent
    failure in the whole protocol."""
    outbound, inbound = a_chat(is_outbound=True), a_chat(is_outbound=False)
    assert (outbound.out_x, outbound.in_x) == (0, 8)
    assert (inbound.out_x, inbound.in_x) == (8, 0)


# --- persistence --------------------------------------------------------------


def test_a_chat_round_trips_through_its_record():
    """data-model.md §5: what ``save`` writes is what ``load`` gives back."""
    chat = a_chat()
    chat.ttl = 30
    chat.layer = 143
    chat.in_seq_no, chat.out_seq_no = 4, 6
    chat.pending_key = OTHER_KEY
    chat.exchange_id = 7788

    back = SecretChat.from_record(chat.to_record())
    assert back.to_record() == chat.to_record()
    assert back.key == KEY and back.key_fingerprint == chat.key_fingerprint
    assert back.pending_key == OTHER_KEY and back.state is ChatState.READY


def test_the_record_carries_every_field_the_data_model_lists():
    """A field added to the entity and forgotten here is a field that silently stops
    surviving a restart."""
    required = {
        "id",
        "access_hash",
        "peer_user_id",
        "is_outbound",
        "state",
        "key",
        "key_fingerprint",
        "pending_key",
        "exchange_id",
        "in_seq_no",
        "out_seq_no",
        "layer",
        "ttl",
        "created_at",
        "rekeyed_at",
        "messages_since_rekey",
        "admin_id",
        "participant_id",
    }
    assert required <= set(a_chat().to_record())


def test_a_chat_never_prints_its_key():
    """Principle IV: "Never included in a repr, str, a log line, an exception, or
    any returned value"."""
    chat = a_chat()
    chat.pending_key = OTHER_KEY
    rendered = repr(chat) + str(chat)
    assert KEY.hex()[:32] not in rendered.lower()
    assert OTHER_KEY.hex()[:32] not in rendered.lower()
    assert str(list(KEY[:8])) not in rendered
