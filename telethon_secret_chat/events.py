"""What the application is told - contracts/public-api.md §3.

Seven events, each naming its chat. Two rules run through all of them.

**A failure is an event, not an exception.** The spec's Assumptions: "a failed
decrypt is a typed event delivered to the application, not an exception thrown
through its update loop." An exception raised inside Telethon's update dispatch
reaches an application as a traceback from a library it did not call.

**An event carries a shape, never a value** (Principle IV). ``DecryptFailed`` is the
one that matters: it says which chat and what kind of failure, and never the
ciphertext, never a partial plaintext, and never how far the parse got.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, List, Optional

__all__ = [
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
    "EVENT_TYPES",
]


@dataclass
class ChatRequested:
    """The peer asked for a chat and it is waiting for ``accept``."""

    chat_id: int
    peer_user_id: int


@dataclass
class ChatReady:
    """Established. The fingerprint is included so the application can show the user
    the same value the peer's client shows (§1.5, FR-003)."""

    chat_id: int
    peer_user_id: int
    key_fingerprint: int


@dataclass
class ChatClosedEvent:
    """Terminal, with the reason as a shape.

    Named with the suffix because ``ChatClosed`` is the EXCEPTION in ``errors.py``
    and the two would otherwise collide in the package's namespace. The contract
    calls the event ``ChatClosed``; this is that event.
    """

    chat_id: int
    reason: str


@dataclass
class MessageReceived:
    """Decrypted, in conversation order (FR-009).

    ``media`` is the ``DecryptedMessageMedia`` when the message carried one, kept
    as the protocol object so ``save_file`` can take the message straight back
    (contracts §2). It holds the file's one-time key (§6.1), so an application that
    logs this field logs key material - hence the repr below.
    """

    chat_id: int
    random_id: int
    seq_no: int
    text: str
    entities: Optional[List[Any]] = None
    ttl: int = 0
    media: Optional[Any] = None
    file: Optional[Any] = None
    #: The ``random_id`` this message replies to, or ``None``. The encrypted layer
    #: has no message ids, so a reply points at the random id its sender chose.
    reply_to: Optional[int] = None

    def __repr__(self) -> str:
        return (
            f"MessageReceived(chat_id={self.chat_id}, seq_no={self.seq_no}, "
            f"chars={len(self.text)}, media={self.media is not None})"
        )


@dataclass
class MessageAcknowledged:
    """The peer's counter passed a message this side sent (§3.6's ``in_seq_no``).

    Distinct from the ``send`` call resolving: the spec's Assumptions make ``send``
    resolve when TELEGRAM accepts the ciphertext, and acknowledgement is what says
    the peer processed it.
    """

    chat_id: int
    seq_no: int
    random_ids: List[int] = field(default_factory=list)


@dataclass
class ServiceActionReceived:
    """One of the thirteen (§5). FR-011: reported rather than silently applied.

    ``applied`` says whether the package acted on it as well - a ``SetMessageTTL``
    changes stored state and is still reported, while a ``RequestKey`` is handled
    internally and is also still reported.
    """

    chat_id: int
    action_name: str
    action: Any
    applied: bool = False


@dataclass
class DecryptFailed:
    """A received message was refused (§2.7, §3.4-§3.6).

    ``reason`` is a phrase this package wrote. Never the ciphertext, never a partial
    plaintext: §2.7 calls checks 2 and 3 "the difference between a decryption
    failure and a buffer-length oracle", and an event reporting which check failed
    at which offset would rebuild the oracle those checks close.
    """

    chat_id: int
    reason: str


EVENT_TYPES = (
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
)
