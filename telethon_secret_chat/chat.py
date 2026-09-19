"""The SecretChat entity and its state machine - data-model.md §1.

What a conversation holds, what survives a restart, and which transitions exist.

The invariant the whole file is arranged around: **the key and its fingerprint are
one unit**. §1.5 derives one from the other, so a record holding key A with
fingerprint B decrypts nothing and looks from outside exactly like a peer problem.
There is no setter for either alone - ``adopt_key`` sets both, and ``to_record``
emits both.

The second rule, from data-model.md §1: **terminal means terminal**. The failures
that close a chat are integrity failures (§3.4 parity, §3.6 monotonicity), and
reviving a closed chat would mean continuing on the counters whose integrity just
failed.
"""

from __future__ import annotations

import time
from enum import Enum
from typing import Any, Dict, Optional

from .crypto import KEY_LENGTH, key_fingerprint
from .errors import ChatClosed, ChatNotReady
from .framing import INITIAL_REMOTE_LAYER

__all__ = ["ChatState", "SecretChat"]


class ChatState(str, Enum):
    """data-model.md §1. ``str`` so a record round-trips through JSON unchanged."""

    REQUESTED = "requested"  # we asked; no key yet
    PENDING = "pending"  # the peer asked; we have not answered
    READY = "ready"  # a key exists and both counters are live
    REKEYING = "rekeying"  # §4 exchange in flight; two keys are held
    CLOSED = "closed"  # terminal


# The whole machine, as data: data-model.md §1's diagram, one row per source state.
_TRANSITIONS = {
    ChatState.REQUESTED: {ChatState.READY, ChatState.CLOSED},
    ChatState.PENDING: {ChatState.READY, ChatState.CLOSED},
    # FR-013: sending still works while rekeying, so READY <-> REKEYING is a cycle
    # rather than a one-way door - §4.6 allows an exchange to be aborted back.
    ChatState.READY: {ChatState.REKEYING, ChatState.CLOSED},
    ChatState.REKEYING: {ChatState.READY, ChatState.CLOSED},
    ChatState.CLOSED: set(),
}


class SecretChat:
    """One conversation with one peer.

    Plain attributes rather than a dataclass, because two of them are key material
    and a dataclass's generated ``repr`` prints every field it has - Principle IV's
    failure mode arriving by default.
    """

    def __init__(
        self,
        *,
        id: int,
        access_hash: int,
        peer_user_id: int,
        is_outbound: bool,
        admin_id: Optional[int] = None,
        participant_id: Optional[int] = None,
    ):
        self.id = id
        self.access_hash = access_hash
        self.peer_user_id = peer_user_id
        # Did THIS side call requestEncryption. §1.6 makes it the only per-side
        # asymmetry in the protocol: it selects the crypto `x` (§2.5), the seq_no
        # parity (§3.4), and which end computes the fingerprint and which compares
        # it (§1.4).
        self.is_outbound = is_outbound
        self.state = ChatState.REQUESTED if is_outbound else ChatState.PENDING

        self.key: Optional[bytes] = None
        self.key_fingerprint: Optional[int] = None
        self.pending_key: Optional[bytes] = None  # §4.3: the key B has derived
        # §4.8: "the previous key may be kept until there are no gaps in received
        # messages up to the switch". Dropping it early makes a message still in
        # flight undecryptable.
        self.previous_key: Optional[bytes] = None
        self.exchange_id: Optional[int] = None
        self.exchange_secret: Optional[int] = None  # this side's `a`/`b` for §4.2
        # Which commitment this side has made, for §4.6's point of no return:
        # None, "requested", "accepted" or "committed".
        self.rekey_role: Optional[str] = None
        # §4.2: "the same Diffie-Hellman parameters (p,g) ... are used. They do not
        # need to be re-transmitted explicitly" - so they have to survive a restart,
        # or a chat that restarts can never rekey again.
        self.dh_prime: Optional[int] = None
        self.dh_g: Optional[int] = None

        # §3.4: "(out_seq_no, in_seq_no) := (0,0), and incremented strictly by 1
        # after any message (service or not) is sent/received and processed."
        self.in_seq_no = 0  # messages received and processed; the next expected
        self.out_seq_no = 0  # messages sent; §3.6's D + 1
        # The peer's echo of OUR counter, kept so §3.6 can check it is
        # non-decreasing across messages rather than only within one.
        self.peer_in_seq_no = 0
        # §3.7: "if the remote client keeps sending out of sync messages, they
        # should be put into the queue without sending a new request". One request
        # per hole, and this is what makes it one.
        self.gap_requested = False
        self.layer = INITIAL_REMOTE_LAYER  # §7.2: starts at 46, only rises
        self.ttl = 0  # §5.1: 0 disables

        now = time.time()
        self.created_at = now
        self.rekeyed_at = now
        self.messages_since_rekey = 0  # §4.1's 100-message trigger
        self.admin_id = admin_id
        self.participant_id = participant_id
        self.closed_reason: Optional[str] = None

    # --- the per-side asymmetry (§2.5) ---------------------------------------

    @property
    def out_x(self) -> int:
        """§2.5: "x=0 for messages from the originator of the secret chat, x=8 for
        the messages in the opposite direction"."""
        return 0 if self.is_outbound else 8

    @property
    def in_x(self) -> int:
        """The mirror. Both index the same 256-byte key, so the two directions never
        share an ``aes_key``."""
        return 8 if self.is_outbound else 0

    # --- the state machine ----------------------------------------------------

    def transition_to(self, target: ChatState) -> None:
        if self.state is ChatState.CLOSED:
            raise ChatClosed(chat_id=self.id, reason=self.closed_reason or "already closed")
        if target not in _TRANSITIONS[self.state]:
            raise ValueError(f"a chat cannot go from {self.state.value!r} to {target.value!r}")
        self.state = target

    def close(self, reason: str) -> None:
        """Terminal, and idempotent - a chat closed twice keeps the FIRST reason,
        because that is the one describing what actually failed."""
        if self.state is ChatState.CLOSED:
            return
        self.state = ChatState.CLOSED
        self.closed_reason = reason

    def require_sendable(self) -> None:
        """contracts/public-api.md §2: ``send_message`` refuses outside ready and
        rekeying. FR-013 puts ``rekeying`` on the allowed side - §4.8 keeps the old
        key in use until each side's own commit point, so a message issued during an
        exchange is delivered rather than dropped."""
        if self.state is ChatState.CLOSED:
            raise ChatClosed(chat_id=self.id, reason=self.closed_reason or "the chat is closed")
        if self.state not in (ChatState.READY, ChatState.REKEYING):
            raise ChatNotReady(chat_id=self.id, state=self.state.value)

    # --- the key --------------------------------------------------------------

    def adopt_key(self, key: bytes) -> None:
        """Take a new current key and its fingerprint together, and become ready.

        The only way either field is written. data-model.md §1: "A backend that
        persists them in separate writes can be interrupted between the two, leaving
        a chat whose fingerprint does not match its key."
        """
        if len(key) != KEY_LENGTH:
            raise ValueError(f"a shared key is exactly {KEY_LENGTH} bytes: §1.4 pads it to that")
        self.key = key
        self.key_fingerprint = key_fingerprint(key)
        if self.state in (ChatState.REQUESTED, ChatState.PENDING):
            self.state = ChatState.READY

    # --- persistence (data-model.md §5) ---------------------------------------

    _FIELDS = (
        "id",
        "access_hash",
        "peer_user_id",
        "is_outbound",
        "key",
        "key_fingerprint",
        "pending_key",
        "previous_key",
        "exchange_id",
        "exchange_secret",
        "rekey_role",
        "dh_prime",
        "dh_g",
        "in_seq_no",
        "out_seq_no",
        "peer_in_seq_no",
        "gap_requested",
        "layer",
        "ttl",
        "created_at",
        "rekeyed_at",
        "messages_since_rekey",
        "admin_id",
        "participant_id",
        "closed_reason",
    )

    def to_record(self) -> Dict[str, Any]:
        """Everything that survives a restart, as one dict. The backend writes it as
        one unit or not at all (data-model.md §5)."""
        record = {name: getattr(self, name) for name in self._FIELDS}
        record["state"] = self.state.value
        return record

    @classmethod
    def from_record(cls, record: Dict[str, Any]) -> "SecretChat":
        chat = cls(
            id=record["id"],
            access_hash=record["access_hash"],
            peer_user_id=record["peer_user_id"],
            is_outbound=record["is_outbound"],
            admin_id=record.get("admin_id"),
            participant_id=record.get("participant_id"),
        )
        for name in cls._FIELDS:
            if name in record:
                setattr(chat, name, record[name])
        chat.state = ChatState(record["state"])
        return chat

    # --- Principle IV ---------------------------------------------------------

    def __repr__(self) -> str:
        """Shape only. §8.8 recorded the archived package interpolating its chat
        repr into five log lines; this one names the chat, its state and whether a
        key exists, and never what the key is."""
        return (
            f"SecretChat(id={self.id}, state={self.state.value!r}, "
            f"peer={self.peer_user_id}, outbound={self.is_outbound}, "
            f"layer={self.layer}, keyed={self.key is not None})"
        )

    __str__ = __repr__
