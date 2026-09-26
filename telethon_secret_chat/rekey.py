"""Perfect forward secrecy - protocol-reference.md §4.

"Please note that your client must support Forward Secrecy in Secret Chats to be
compatible with official Telegram clients", so this is compatibility, not a luxury.

The whole exchange is four actions and one window:

    A: RequestKey(exchange_id, g_a)   ->
                                      <- B: AcceptKey(exchange_id, g_b, fingerprint)
    A: CommitKey(exchange_id, fingerprint) ->
                                      <- B: (Noop, if it has nothing else to send)

and between them, §4.8's **two-key window**: each side keeps encrypting with the OLD
key until its own commit point, and a receiver holds up to two keys and picks by the
``key_fingerprint`` prefix of §2.6. Dropping the old key early makes a message still
in flight undecryptable, which is the failure §8.4 found in the archived package -
it overwrote ``peer.auth_key`` outright.

Three more things §8.4 measured there, each fixed at the site below: the
``exchange_id`` came from ``random.randint`` over ~27 bits; ``AbortKey`` was never
handled on receipt, so a peer's abort left the chat half-open forever; and the
``CommitKey`` fingerprint check compared bytes against an int behind a guard that
returned first, so it was dead code.

§4.8 also says what must NOT change: "seq_no counters are not reset by a rekey".
"""

from __future__ import annotations

import secrets
import time

from . import dh, handshake
from .crypto import key_fingerprint
from .errors import ParameterRejected
from .schema import secret_tl as tl

__all__ = [
    "should_rekey",
    "new_exchange_id",
    "resolve_collision",
    "may_abort",
    "select_key",
    "adopt_new_key",
    "retire_previous_key",
    "retire_previous_key_if_settled",
    "start",
    "handle",
    "MESSAGE_TRIGGER",
    "PROTOCOL_ACTIONS",
    "AGE_TRIGGER",
]

# §4.1: "once a key has been used to decrypt and encrypt more than 100 messages, or
# has been in use for more than one week, provided the key has been used to encrypt
# at least one message". TDLib: `last_message_id + 100 < message_id ||
# last_timestamp + 60 * 60 * 24 * 7 < Time::now()`.
MESSAGE_TRIGGER = 100
AGE_TRIGGER = 7 * 24 * 60 * 60

#: The exchange's own messages. They never evaluate the trigger: a RequestKey is
#: itself sent as a service action, and letting it re-check would start a second
#: exchange from inside the first.
PROTOCOL_ACTIONS = (
    tl.DecryptedMessageActionRequestKey,
    tl.DecryptedMessageActionAcceptKey,
    tl.DecryptedMessageActionCommitKey,
    tl.DecryptedMessageActionAbortKey,
    tl.DecryptedMessageActionNoop,
)


def should_rekey(chat, now: float) -> bool:
    """§4.1's trigger, with both of TDLib's guards.

    "you should never initiate a new instance of the re-keying protocol if an
    uncompleted instance exists, initiated by either party" - and TDLib adds
    ``other_auth_key.empty()``, because a chat still holding the previous key has
    not finished the last one.
    """
    if chat.exchange_id is not None or chat.previous_key is not None:
        return False
    if chat.messages_since_rekey <= 0:
        # "provided the key has been used to encrypt at least one message".
        return False
    return chat.messages_since_rekey > MESSAGE_TRIGGER or chat.rekeyed_at + AGE_TRIGGER < now


def new_exchange_id() -> int:
    """§4.2: "a random number identifying this instance of the Re-Keying Protocol".

    A full-width signed int64 from ``secrets``. §4.7's tie-break rests on collisions
    having probability 2^-64, and §8.4 measured the archived package drawing ~27
    bits from the non-cryptographic ``random`` module - which makes both the
    collision assumption and the unpredictability false.
    """
    return secrets.randbits(64) - (1 << 63)


def resolve_collision(mine: int, theirs: int) -> str:
    """§4.7: both sides sent ``RequestKey`` at once.

    Without a rule "the re-keying will never happen", because each side would abort
    for being already in an exchange. Compared "as a long, i.e. signed little-endian
    64-bit integer".
    """
    if mine > theirs:
        # "silently abandon the newly-suggested instance, send no AbortKey".
        return "abandon_theirs"
    if mine < theirs:
        # "answer the received RequestKey with AcceptKey and participate only in the
        # peer's instance".
        return "join_theirs"
    # Probability 2^-64: "abort both instances without sending an explicit
    # decryptedMessageActionAbortKey. The other side will do the same."
    return "abort_both"


def may_abort(chat) -> bool:
    """§4.6: "unless decryptedMessageActionCommitKey or decryptedMessageActionAcceptKey
    has been already sent by the party in question".

    §4.3 states the same as B's point of no return: "Once side B sends
    decryptedMessageActionAcceptKey, it cannot abort the key exchange; it must be
    ready to switch to the new key immediately".
    """
    return chat.exchange_id is not None and chat.rekey_role not in ("accepted", "committed")


def select_key(chat, fingerprint: int):
    """§2.6 and §4.5: pick the held key the frame names, or ``None``.

    Three can be live at once. The current key; the previous one, retained until the
    gaps before the switch are filled (§4.8); and the pending one, because §4.5 says
    a message encrypted under the NEW key is itself the signal that the peer has
    switched - "it may happen that the decryptedMessageActionCommitKey has been lost
    and will be re-requested later".
    """
    for candidate in (chat.key, chat.pending_key, chat.previous_key):
        if candidate is not None and key_fingerprint(candidate) == fingerprint:
            return candidate
    return None


def adopt_new_key(chat, key: bytes) -> None:
    """Switch to the new key and KEEP the old one (§4.8).

    "the previous key may be kept until there are no gaps in received messages up to
    the switch to the new key. Once all the gaps have been filled, the old key must
    be securely discarded."

    The counters are deliberately untouched: §4.8 records that nothing in the PFS or
    seq_no pages resets them, and TDLib's ``seq_no_state_`` is independent of
    ``pfs_state_``. Resetting them would put every later message at a position the
    peer has already passed, and §3.5 drops those silently.
    """
    chat.previous_key = chat.key
    chat.adopt_key(key)
    chat.pending_key = None
    chat.exchange_id = None
    chat.exchange_secret = None
    chat.rekey_role = None
    chat.new_key_confirmed = False
    chat.rekeyed_at = time.time()
    chat.messages_since_rekey = 0


def retire_previous_key_if_settled(chat) -> None:
    """§4.8: discard the old key once there is nothing left that could need it.

    "the previous key may be kept until there are no gaps in received messages up to
    the switch to the new key. Once all the gaps have been filled, the old key must
    be securely discarded."

    So the condition is the gap queue, not a timer: while a hole is open, a message
    written before the switch may still arrive, and it can only be read with the key
    that is about to be thrown away.
    """
    if chat.previous_key is not None and chat.new_key_confirmed and not chat.gap_requested:
        retire_previous_key(chat)


def retire_previous_key(chat) -> None:
    """ "Once all the gaps have been filled, the old key must be securely discarded.\" """
    chat.previous_key = None


# --- driving the exchange -----------------------------------------------------


async def start(manager, chat) -> None:
    """Publish the request, secret and outbox record in one durable transition."""
    if chat.exchange_id is not None or chat.previous_key is not None:
        return
    exchange_id = new_exchange_id()
    secret = handshake.generate_secret()
    g_a = handshake.public_value(chat.dh_g, secret, chat.dh_prime)

    def prepared():
        chat.exchange_id = exchange_id
        chat.exchange_secret = secret
        chat.rekey_role = "requested"
        chat.state = type(chat.state).REKEYING

    await manager._send_action(
        chat,
        tl.DecryptedMessageActionRequestKey(exchange_id=exchange_id, g_a=g_a.to_bytes(256, "big")),
        after_prepare=prepared,
    )


async def handle(manager, chat, action):
    """The inbound half of §4.2-§4.6."""
    from .actions import Outcome

    if isinstance(action, tl.DecryptedMessageActionRequestKey):
        await _on_request(manager, chat, action)
        # An unsafe request closes the chat instead of being applied.
        return Outcome(applied=chat.state.value != "closed")
    if isinstance(action, tl.DecryptedMessageActionAcceptKey):
        await _on_accept(manager, chat, action)
        return Outcome(applied=True)
    if isinstance(action, tl.DecryptedMessageActionCommitKey):
        await _on_commit(manager, chat, action)
        return Outcome(applied=True)
    if isinstance(action, tl.DecryptedMessageActionAbortKey):
        # §4.6: "Receiving it must clear the local exchange state." §8.4: the
        # archived package let this fall through to the application, so the chat
        # could sit in a half-open exchange indefinitely.
        if chat.exchange_id == action.exchange_id:
            _clear(chat)
        return Outcome(applied=True)
    return Outcome(applied=False)


async def _on_request(manager, chat, action) -> None:
    """Only simultaneous *requests* are eligible for the signed-ID tie break."""
    if chat.previous_key is not None:
        return
    if chat.exchange_id is not None:
        if chat.rekey_role != "requested":
            return  # An accepted exchange cannot be abandoned or overwritten.
        outcome = resolve_collision(chat.exchange_id, action.exchange_id)
        if outcome == "abandon_theirs":
            return
        if outcome == "abort_both":
            _clear(chat)
            return

    g_a = dh.value_from_bytes(action.g_a)
    b = handshake.generate_secret()
    try:
        key = handshake.shared_key(peer_value=g_a, secret=b, p=chat.dh_prime, chat_id=chat.id)
    except ParameterRejected as failure:
        # TDLib treats this as fatal (`on_inbound_action(RequestKey)` -> `cancel_chat`).
        # Raising instead left the action at the head of the delivery mailbox, and
        # every later message queued behind it undelivered.
        await manager.close(chat.id, failure.reason)
        return
    g_b = handshake.public_value(chat.dh_g, b, chat.dh_prime)

    def prepared():
        chat.exchange_id = action.exchange_id
        chat.exchange_secret = None
        chat.pending_key = key
        chat.rekey_role = "accepted"
        chat.state = type(chat.state).REKEYING

    await manager._send_action(
        chat,
        tl.DecryptedMessageActionAcceptKey(
            exchange_id=action.exchange_id,
            g_b=g_b.to_bytes(256, "big"),
            key_fingerprint=key_fingerprint(key),
        ),
        after_prepare=prepared,
    )


async def _on_accept(manager, chat, action) -> None:
    """§4.4: A checks, commits, and from then on encrypts with the new key."""
    if chat.exchange_id != action.exchange_id or chat.exchange_secret is None:
        return
    g_b = dh.value_from_bytes(action.g_b)
    try:
        key = handshake.shared_key(
            peer_value=g_b, secret=chat.exchange_secret, p=chat.dh_prime, chat_id=chat.id
        )
        # §4.3: the fingerprint is "used as a sanity check of the implementation".
        # A real check, unlike §8.4's dead comparison of bytes against an int.
        handshake.verify_fingerprint(chat_id=chat.id, key=key, claimed=action.key_fingerprint)
    except ParameterRejected:
        # §4.6: "the received values of g_a, g_b and other parameters do not pass
        # security checks" is explicit grounds to abort the exchange - and this side
        # has sent neither Accept nor Commit, so it still may.
        await _abort(manager, chat)
        return

    exchange_id = chat.exchange_id

    def prepared():
        adopt_new_key(chat, key)
        chat.state = type(chat.state).READY

    # The commit is serialized/encrypted using the OLD key. Adoption and its
    # exact old-key ciphertext are then persisted together, before the RPC.
    await manager._send_action(
        chat,
        tl.DecryptedMessageActionCommitKey(
            exchange_id=exchange_id, key_fingerprint=key_fingerprint(key)
        ),
        after_prepare=prepared,
    )


async def _on_commit(manager, chat, action) -> None:
    """§4.5: B switches, and sends a ``Noop`` so A may retire the old key."""
    if chat.exchange_id != action.exchange_id or chat.pending_key is None:
        return
    if key_fingerprint(chat.pending_key) != action.key_fingerprint:
        # The same sanity check from the other side. A mismatch here means the two
        # ends did not derive the same key, so switching would break the chat.
        await _abort(manager, chat)
        return
    key = chat.pending_key

    def prepared():
        adopt_new_key(chat, key)
        chat.state = type(chat.state).READY
        # B may retire after the authenticated CommitKey and all preceding gaps.
        # A, unlike B, must wait for a new-key packet (the Noop below).
        chat.new_key_confirmed = True

    await manager._send_action(
        chat,
        tl.DecryptedMessageActionNoop(),
        after_prepare=prepared,
        encryption_key=key,
    )


async def _abort(manager, chat) -> None:
    exchange_id = chat.exchange_id
    if may_abort(chat) and exchange_id is not None:
        await manager._send_action(
            chat,
            tl.DecryptedMessageActionAbortKey(exchange_id=exchange_id),
            after_prepare=lambda: _clear(chat),
        )
    elif exchange_id is not None:
        # After acceptance, abandoning the key would violate the protocol.
        # A failed commit sanity check must terminate the chat instead.
        await manager.close(chat.id, "rekey commitment failed its fingerprint check")


def _clear(chat) -> None:
    """Back to a chat with one key and no exchange. US6 scenario 3: "a stated,
    recoverable condition rather than silently using a half-swapped key"."""
    chat.exchange_id = None
    chat.pending_key = None
    chat.exchange_secret = None
    chat.rekey_role = None
    if chat.state.value == "rekeying":
        chat.transition_to(type(chat.state).READY)
