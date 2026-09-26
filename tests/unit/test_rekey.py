"""protocol-reference.md §4 - perfect forward secrecy, and the two-key window.

"Please note that your client must support Forward Secrecy in Secret Chats to be
compatible with official Telegram clients."

The parts that are easy to get wrong, and that §8.4 measured the archived package
getting wrong:

- **`exchange_id`** was ``random.randint(10000000, 99999999)`` - the non-cryptographic
  module, and ~27 bits of an int64 field, so §4.7's 2^-64 collision assumption did
  not hold.
- **`AbortKey` was never handled on receipt**, so a peer's abort never cleared the
  local exchange and the chat could sit half-open forever.
- **The previous key was overwritten outright**, so a message still in flight under
  the old key became undecryptable - which §4.8 exists to prevent.
- **The `CommitKey` fingerprint check compared bytes against an int**, so it could
  never be equal, and the guard above it returned early anyway: dead code, and the
  fingerprint effectively unchecked.
"""

import time

import pytest

from telethon_secret_chat import SecretChatManager, rekey
from telethon_secret_chat.chat import ChatState, SecretChat
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import FileStorage, MemoryStorage

from .fake_client import Wire, establish

KEY = bytes((i * 5 + 3) % 256 for i in range(256))


@pytest.fixture
async def pair():
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    yield wire, a, b
    await a.stop()
    await b.stop()


def a_chat(**over):
    chat = SecretChat(id=7, access_hash=1, peer_user_id=2, is_outbound=True)
    chat.adopt_key(KEY)
    for name, value in over.items():
        setattr(chat, name, value)
    return chat


# --- §4.1 the trigger ---------------------------------------------------------


def test_a_fresh_chat_does_not_rekey():
    assert not rekey.should_rekey(a_chat(), now=time.time())


def test_a_hundred_messages_triggers_it():
    """§4.1: "once a key has been used to decrypt and encrypt more than 100
    messages". TDLib: ``pfs_state_.last_message_id + 100 < seq_no_state_.message_id``."""
    assert not rekey.should_rekey(a_chat(messages_since_rekey=100), now=time.time())
    assert rekey.should_rekey(a_chat(messages_since_rekey=101), now=time.time())


def test_a_week_triggers_it():
    """ "or has been in use for more than one week, provided the key has been used to
    encrypt at least one message"."""
    now = time.time()
    old = now - 8 * 24 * 3600
    assert rekey.should_rekey(a_chat(rekeyed_at=old, messages_since_rekey=1), now=now)


def test_an_old_but_unused_key_does_not_trigger_it():
    """ "provided the key has been used to encrypt at least one message". A chat
    nobody has spoken in does not need new keys."""
    now = time.time()
    chat = a_chat(rekeyed_at=now - 30 * 24 * 3600, messages_since_rekey=0)
    assert not rekey.should_rekey(chat, now=now)


def test_an_exchange_already_running_does_not_trigger_another():
    """§4.1: "you should never initiate a new instance of the re-keying protocol if
    an uncompleted instance exists, initiated by either party"."""
    chat = a_chat(messages_since_rekey=500, exchange_id=12345)
    assert not rekey.should_rekey(chat, now=time.time())


def test_a_retained_previous_key_does_not_trigger_another():
    """TDLib's fourth condition: ``pfs_state_.other_auth_key.empty()``. A chat still
    holding the old key has not finished the last exchange."""
    chat = a_chat(messages_since_rekey=500, previous_key=KEY)
    assert not rekey.should_rekey(chat, now=time.time())


# --- §4.2 the exchange id -----------------------------------------------------


def test_the_exchange_id_is_a_full_width_random_int64():
    """§4.7's tie-break assumes a collision probability of 2^-64, which only holds if
    the id really is 64 bits. §8.4 measured ~27 bits from ``random.randint``."""
    ids = [rekey.new_exchange_id() for _ in range(200)]
    assert len(set(ids)) == 200
    assert all(-(2**63) <= i < 2**63 for i in ids)
    assert max(abs(i) for i in ids).bit_length() > 40


def test_the_exchange_id_does_not_come_from_the_random_module(monkeypatch):
    import random as insecure

    monkeypatch.setattr(
        insecure, "randint", lambda *a: pytest.fail("the `random` module was used")
    )
    monkeypatch.setattr(
        insecure, "getrandbits", lambda *a: pytest.fail("the `random` module was used")
    )
    rekey.new_exchange_id()


# --- §4.7 concurrent exchanges ------------------------------------------------


@pytest.mark.parametrize(
    "mine, theirs, expected",
    [
        (100, 50, "abandon_theirs"),  # mine is larger: silently drop the new instance
        (50, 100, "join_theirs"),  # mine is smaller: answer their RequestKey
        (50, 50, "abort_both"),  # equal, 2^-64: abort both, silently
    ],
)
def test_the_documented_tie_break(mine, theirs, expected):
    """§4.7, comparing "as a long, i.e. signed little-endian 64-bit integer".

    Without it, both sides abort because each is already in an exchange, and "the
    re-keying will never happen".
    """
    assert rekey.resolve_collision(mine, theirs) == expected


def test_no_abort_is_sent_for_either_collision_outcome():
    """§4.7 is explicit for both: the larger side abandons "without sending an
    explicit decryptedMessageActionAbortKey", and on equality "abort both instances
    without sending an explicit decryptedMessageActionAbortKey"."""
    assert rekey.sends_abort_on_collision() is False


# --- §4.6 the point of no return ----------------------------------------------


def test_an_exchange_may_be_aborted_before_a_commitment():
    assert rekey.may_abort(a_chat(exchange_id=1, rekey_role=None))


@pytest.mark.parametrize("role", ["accepted", "committed"])
def test_an_exchange_may_not_be_aborted_after_one(role):
    """§4.6: "unless decryptedMessageActionCommitKey or decryptedMessageActionAcceptKey
    has been already sent by the party in question". §4.3: "Once side B sends
    decryptedMessageActionAcceptKey, it cannot abort the key exchange"."""
    assert not rekey.may_abort(a_chat(exchange_id=1, rekey_role=role))


# --- §4.8 the two-key window --------------------------------------------------


def test_a_frame_is_matched_to_whichever_held_key_it_names():
    """§2.6 and §4.8: "a receiver holds up to two keys and selects by the
    key_fingerprint prefix". TDLib compares against ``pfs_state_.auth_key.id()``
    then ``pfs_state_.other_auth_key.id()``."""
    from telethon_secret_chat import crypto

    other = bytes((i * 9 + 1) % 256 for i in range(256))
    chat = a_chat(previous_key=other)
    assert rekey.select_key(chat, crypto.key_fingerprint(KEY)) == KEY
    assert rekey.select_key(chat, crypto.key_fingerprint(other)) == other
    assert rekey.select_key(chat, 999) is None


def test_the_pending_key_can_decrypt_too():
    """§4.5: B switches when it sees "a decryptedMessageActionCommitKey **or a
    message encrypted by the new key, recognized by the value of key_fingerprint**
    (it may happen that the decryptedMessageActionCommitKey has been lost)"."""
    from telethon_secret_chat import crypto

    new = bytes((i * 3 + 7) % 256 for i in range(256))
    chat = a_chat(pending_key=new, exchange_id=5)
    assert rekey.select_key(chat, crypto.key_fingerprint(new)) == new


def test_the_counters_are_not_reset_by_a_rekey():
    """§4.8: "seq_no counters are NOT reset by a rekey: nothing in the PFS or seq_no
    pages resets them, and TDLib's seq_no_state_ is independent of pfs_state_." A
    reset would put every later message where the peer has already been."""
    chat = a_chat(in_seq_no=17, out_seq_no=23)
    rekey.adopt_new_key(chat, bytes((i * 11) % 256 for i in range(256)))
    assert (chat.in_seq_no, chat.out_seq_no) == (17, 23)


def test_adopting_a_new_key_retains_the_old_one():
    """§4.8: "A may only discard the previous key after a message encrypted with the
    new key has been received." Overwriting it makes any message still in flight
    undecryptable - §8.4 measured exactly that."""
    new = bytes((i * 11) % 256 for i in range(256))
    chat = a_chat()
    rekey.adopt_new_key(chat, new)
    assert chat.key == new and chat.previous_key == KEY


def test_the_previous_key_is_dropped_once_the_new_one_has_carried_a_message():
    new = bytes((i * 11) % 256 for i in range(256))
    chat = a_chat()
    rekey.adopt_new_key(chat, new)
    rekey.retire_previous_key(chat)
    assert chat.previous_key is None


# --- end to end ---------------------------------------------------------------


async def test_an_exchange_completes_and_both_ends_agree_on_the_new_key(pair):
    """US6 scenario 1: "a new key is negotiated and both ends keep decrypting each
    other"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    before = chat_a.key

    await a.rekey(chat_a.id)

    assert a.status(chat_a.id).key != before
    assert a.status(chat_a.id).key == b.status(chat_b.id).key
    assert a.status(chat_a.id).key_fingerprint == b.status(chat_b.id).key_fingerprint


async def test_the_conversation_continues_after_the_exchange(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", lambda e: got.append(e.text))

    await a.send_message(chat_a.id, "before")
    await a.rekey(chat_a.id)
    await a.send_message(chat_a.id, "after")
    await b.send_message(chat_b.id, "and back")

    assert got == ["before", "after"]
    assert a.status(chat_a.id).state is ChatState.READY


async def test_a_message_sent_mid_exchange_is_delivered(pair):
    """US6 scenario 2 and FR-013: "a message sent during an exchange MUST be
    delivered rather than dropped". §4.8: both sides keep encrypting with the OLD
    key until their own commit point."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", lambda e: got.append(e.text))

    # Hold B's replies so the exchange is genuinely in flight while A sends.
    wire.b.hold = True
    await a.rekey(chat_a.id)
    assert a.status(chat_a.id).state is ChatState.REKEYING
    await a.send_message(chat_a.id, "sent while rekeying")
    await wire.b.release()

    assert "sent while rekeying" in got


async def test_a_peers_abort_clears_the_local_exchange(pair):
    """§4.6: "Receiving it must clear the local exchange state." §8.4 measured the
    archived package letting AbortKey fall through to the application, so
    ``peer.rekeying`` was never cleared and the chat deadlocked half-open."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    wire.b.hold = True
    await a.rekey(chat_a.id)
    exchange_id = a.status(chat_a.id).exchange_id

    await b._send(
        b.status(chat_b.id),
        tl.DecryptedMessageService(
            random_id=1, action=tl.DecryptedMessageActionAbortKey(exchange_id=exchange_id)
        ),
    )
    wire.b.hold = False
    await wire.b.release()

    chat = a.status(chat_a.id)
    assert chat.exchange_id is None and chat.pending_key is None
    assert chat.state is ChatState.READY, "US6 scenario 3: a stated, recoverable condition"


async def test_a_bad_fingerprint_in_an_accept_does_not_switch_the_key(pair):
    """§4.3: the fingerprint is "used as a sanity check of the implementation". §8.4
    measured the archived package's equivalent check comparing bytes to an int - it
    could never be equal, and a guard above it made the comparison dead code."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    # A's RequestKey is held, so B never joins the exchange and never answers it
    # honestly - the only AcceptKey A sees is the forged one below.
    wire.a.hold = True
    await a.rekey(chat_a.id)
    before = a.status(chat_a.id).key

    await b._send(
        b.status(chat_b.id),
        tl.DecryptedMessageService(
            random_id=1,
            action=tl.DecryptedMessageActionAcceptKey(
                exchange_id=a.status(chat_a.id).exchange_id,
                g_b=(2**2000).to_bytes(256, "big"),  # a well-formed value in range
                key_fingerprint=1,  # that is not the fingerprint of the key it implies
            ),
        ),
    )
    assert a.status(chat_a.id).key == before, "a mismatched fingerprint switched the key"
    assert a.status(chat_a.id).pending_key is None
    assert a.status(chat_a.id).state is ChatState.READY, "the chat was left mid-exchange"


# --- T046: a restart during an exchange ---------------------------------------


async def test_a_restart_during_an_exchange_leaves_the_chat_usable(tmp_path):
    """The spec's edge case: "The application restarts mid-exchange: on restart the
    chat is either usable or reported unusable - never silently on the wrong key.
    **Both keys and the pending queue survive a restart.**\" """
    wire = Wire()
    a = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    chat_a, chat_b = await establish(a, b, wire)

    wire.b.hold = True
    await a.rekey(chat_a.id)
    in_flight = a.status(chat_a.id)
    assert in_flight.state is ChatState.REKEYING and in_flight.exchange_id is not None
    await a.stop()

    revived = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    await revived.start()
    after = revived.status(chat_a.id)

    assert after.key == in_flight.key, "the CURRENT key changed across a restart"
    assert after.exchange_id == in_flight.exchange_id, "the exchange was forgotten"
    assert after.state is ChatState.REKEYING
    # And the exchange can still finish, which is what "usable" means here.
    await wire.b.release()
    assert revived.status(chat_a.id).state is ChatState.READY
    await revived.stop()
    await b.stop()


# --- the trigger actually fires, and the old key actually goes ----------------


async def test_the_documented_trigger_rekeys_without_being_asked(pair):
    """The spec's Assumptions: "the package rekeys automatically on the documented
    trigger... It is not left to the caller to remember." A `should_rekey` that
    nothing calls is a policy nobody applies.
    """
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    before = chat_a.key

    # The state a chat is in once the key has carried more than 100 messages -
    # §4.1 is "MORE than 100", and TDLib's `last_message_id + 100 < message_id` is
    # the same strict comparison. The next send is the one that must notice.
    a.status(chat_a.id).messages_since_rekey = rekey.MESSAGE_TRIGGER + 1
    await a.send_message(chat_a.id, "the one that notices")

    assert a.status(chat_a.id).key != before, "the 100-message trigger never fired"
    assert a.status(chat_a.id).key == b.status(chat_b.id).key


async def test_the_previous_key_is_discarded_once_the_new_one_is_in_use(pair):
    """§4.8: "Once all the gaps have been filled, the old key must be securely
    discarded." Holding it forever is holding key material the forward secrecy of
    this exchange was supposed to retire."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    await a.send_message(chat_a.id, "first")
    await a.rekey(chat_a.id)

    # Both ends switched; now traffic flows under the new key with no gaps open.
    await b.send_message(chat_b.id, "under the new key")
    await a.send_message(chat_a.id, "and back")

    assert a.status(chat_a.id).previous_key is None
    assert b.status(chat_b.id).previous_key is None


async def test_the_old_key_is_kept_while_a_gap_is_open(pair):
    """The other half of the same sentence: "may be kept until there are no gaps in
    received messages up to the switch". A message still in flight under the old key
    is exactly what it is kept for."""
    from telethon_secret_chat import rekey as rk

    chat = a_chat()
    rk.adopt_new_key(chat, bytes((i * 11) % 256 for i in range(256)))
    chat.gap_requested = True
    rk.retire_previous_key_if_settled(chat)
    assert chat.previous_key is not None
    chat.gap_requested = False
    rk.retire_previous_key_if_settled(chat)
    assert chat.previous_key is not None  # The initiator still needs new-key confirmation.
    chat.new_key_confirmed = True
    rk.retire_previous_key_if_settled(chat)
    assert chat.previous_key is None
