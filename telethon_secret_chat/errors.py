"""What this package raises, and the one rule all of it obeys.

Constitution Principle IV: an error carries a SHAPE, never a value. The type name,
the chat it concerns, and a bounded description of what was refused - and never the
key, the plaintext, the ciphertext, a peer's public value, or the ``repr`` of an
object that came off the wire.

That last one is the trap worth naming. Telethon's generated TL types print their
fields, so ``f"rejected: {obj}"`` over anything from the wire is a key in a log
line. Every message here is COMPOSED from parts this module chose; no constructor
accepts an object it would have to format.

The enforcement is ``tests/unit/test_errors_leak_nothing.py``, which drives every
type below with material that would be catastrophic to print and asserts it is
absent - including a catch-all for any long opaque run, which is the shape key
material takes when something formats it by accident.
"""

from __future__ import annotations

from typing import Optional, Tuple

__all__ = [
    "SecretChatError",
    "ParameterRejected",
    "MessageRejected",
    "ChatNotReady",
    "ChatClosed",
    "StorageRequired",
    "LayerUnsupported",
    "ResendUnsatisfiable",
    "StoreCorrupt",
]


class SecretChatError(Exception):
    """Base for everything this package raises.

    One family so an application can catch this package's failures without
    catching the world, and so a new error type cannot quietly escape the leak
    test - which asserts that every ``Exception`` subclass exported here is one it
    covers.

    ``chat_id`` is optional because one failure (a missing storage backend) happens
    before any chat exists.
    """

    def __init__(self, message: str, *, chat_id: Optional[int] = None):
        self.chat_id = chat_id
        # The id is safe to carry: Telegram assigns it, both ends know it, and an
        # operator cannot correlate a refusal with a conversation without it.
        self._message = f"[chat {chat_id}] {message}" if chat_id is not None else message
        super().__init__(self._message)

    def __repr__(self) -> str:
        # Explicit rather than inherited: the default would render the args tuple,
        # and an args tuple is whatever a future constructor happens to put in it.
        return f"{type(self).__name__}({self._message!r})"


class ParameterRejected(SecretChatError):
    """A Diffie-Hellman parameter failed a check the protocol requires (§1.2).

    ``reason`` is a phrase this package wrote - "the prime is not a safe 2048-bit
    prime" - never the value that failed. The value is a 2048-bit integer derived
    from what the server sent, and printing it tells an operator nothing they can
    act on while putting exchange material in a log.
    """

    def __init__(self, *, chat_id: Optional[int] = None, reason: str):
        self.reason = reason
        super().__init__(f"refused the key exchange: {reason}", chat_id=chat_id)


class MessageRejected(SecretChatError):
    """A received message failed one of the receive-side checks (§2.7, §3.4-§3.6).

    Internal to the package, and deliberately not in the public surface of
    ``contracts/public-api.md`` §4: the contract says a failed decrypt reaches the
    application as a ``DecryptFailed`` EVENT, not as an exception thrown through its
    update loop. This is what the manager catches to build that event.

    ``reason`` is a phrase this module wrote. Never the ciphertext, never a partial
    plaintext, and never how far the parse got - "the difference between a
    decryption failure and a buffer-length oracle" is §2.7's own wording, and an
    error that reports which check failed at which offset rebuilds the oracle the
    checks were written to close.
    """

    def __init__(self, *, chat_id: Optional[int] = None, reason: str):
        self.reason = reason
        super().__init__(f"rejected a received message: {reason}", chat_id=chat_id)


class ChatNotReady(SecretChatError):
    """An operation needs an established chat and this one is not established.

    Carries the state name so the caller knows whether to wait for the peer, accept
    an incoming request, or give up.
    """

    def __init__(self, *, chat_id: int, state: str):
        self.state = state
        super().__init__(
            f"the chat is {state!r}; this operation needs an established chat",
            chat_id=chat_id,
        )


class ChatClosed(SecretChatError):
    """The chat is terminal and will not be revived.

    Deliberately not recoverable. The failures that close a chat are integrity
    failures - a sequence number that cannot be reconciled, a parity violation -
    and continuing on those counters would mean continuing on the thing that just
    proved untrustworthy.
    """

    def __init__(self, *, chat_id: int, reason: str):
        self.reason = reason
        super().__init__(f"the chat is closed: {reason}", chat_id=chat_id)


class StorageRequired(SecretChatError):
    """The manager was constructed without a storage backend.

    An error rather than a default, because the default would have to write key
    material somewhere - and a library that picks that location silently picks one
    the operator never protected. FR-014.
    """

    def __init__(self) -> None:
        super().__init__(
            "a storage backend is required: secret-chat keys must be persisted "
            "somewhere the application chose. There is no default, deliberately - "
            "see telethon_secret_chat.storage for the two shipped backends"
        )


class LayerUnsupported(SecretChatError):
    """The peer cannot reach the layer this package needs (§7).

    MTProto 2.0 begins at layer 73 and this package implements nothing below it, so
    a peer stuck lower is refused rather than served by a silent downgrade.
    """

    def __init__(self, *, chat_id: int, peer_layer: int):
        self.peer_layer = peer_layer
        super().__init__(
            f"the peer announced layer {peer_layer}; this package requires 73 or "
            "higher, below which the protocol is MTProto 1.0 and out of scope",
            chat_id=chat_id,
        )


class ResendUnsatisfiable(SecretChatError):
    """The peer asked for messages older than this side still retains (§3.7).

    The protocol's answer to an unsatisfiable resend is to abort the chat, so this
    is raised alongside closing it. The span is safe to state - sequence numbers are
    positions, not content.
    """

    def __init__(self, *, chat_id: int, requested: Tuple[int, int], retained_from: int):
        self.requested = requested
        # The close reason the manager records; every refusal that ends a chat has one.
        self.reason = "a resend request could not be satisfied"
        self.retained_from = retained_from
        start, end = requested
        super().__init__(
            f"cannot resend messages {start}-{end}: this side retains from "
            f"{retained_from}. The protocol requires the chat to end rather than "
            "answer a resend it cannot satisfy",
            chat_id=chat_id,
        )


class StoreCorrupt(SecretChatError):
    """A stored record failed validation at ``start()``, so nothing was installed.

    A truncated key or a fingerprint that no longer matches its key does not crash;
    it fails later, on the first message, looking exactly like a peer problem.
    Refusing the whole start is the one point where an operator can still act.
    ``reason`` names the rule that failed, never the value that failed it.
    """

    def __init__(self, *, chat_id: Optional[int], reason: str):
        self.reason = reason
        super().__init__(f"the secret-chat store is corrupt: {reason}", chat_id=chat_id)
