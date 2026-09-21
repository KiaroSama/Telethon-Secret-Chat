"""The thirteen service actions - protocol-reference.md §5, both directions.

§5 lists thirteen constructors and says the set is closed: "thirteen constructors,
no more". A usable client handles all thirteen on receipt and can SEND at least
5.1-5.4, 5.6, 5.7 and 5.9-5.13. §8.5 measured the archived package with **five of
the six the consumer needs having no send path at all**, and with `AbortKey` falling
through to the application on receipt, so a peer's abort never cleared the local
exchange and the chat could sit half-open indefinitely.

The split is data-model.md §4, and it decides who acts:

- **Conversation** - the application's business. Reported as an event; the one that
  changes stored state (`SetMessageTTL`) changes it here.
- **Protocol** and **Rekey** - the package's business. Handled without asking, and
  still reported (FR-011): "report a service action received from the peer to the
  application rather than silently applying it."

Outbound composition lives here too, so §5 has one home rather than being half in
the manager.
"""

from __future__ import annotations

from typing import NamedTuple

from . import framing
from .schema import secret_tl as tl

__all__ = ["Outcome", "handle", "group_of", "CONVERSATION", "PROTOCOL", "REKEY"]

# data-model.md §4, as three tuples so `group_of` cannot disagree with the table.
CONVERSATION = (
    tl.DecryptedMessageActionSetMessageTTL,  # 5.1
    tl.DecryptedMessageActionReadMessages,  # 5.2
    tl.DecryptedMessageActionDeleteMessages,  # 5.3
    tl.DecryptedMessageActionScreenshotMessages,  # 5.4
    tl.DecryptedMessageActionFlushHistory,  # 5.5
    tl.DecryptedMessageActionTyping,  # 5.8
)
PROTOCOL = (
    tl.DecryptedMessageActionNotifyLayer,  # 5.7
    tl.DecryptedMessageActionResend,  # 5.6
    tl.DecryptedMessageActionNoop,  # 5.13
)
REKEY = (
    tl.DecryptedMessageActionRequestKey,  # 5.9
    tl.DecryptedMessageActionAcceptKey,  # 5.10
    tl.DecryptedMessageActionCommitKey,  # 5.11
    tl.DecryptedMessageActionAbortKey,  # 5.12
)


class Outcome(NamedTuple):
    """Whether the package acted, as well as reported."""

    applied: bool


def group_of(action) -> str:
    """Which of data-model.md §4's three groups an action belongs to."""
    if isinstance(action, CONVERSATION):
        return "conversation"
    if isinstance(action, PROTOCOL):
        return "protocol"
    if isinstance(action, REKEY):
        return "rekey"
    return "unknown"


async def handle(manager, chat, action) -> Outcome:
    """Act on an inbound action where the package should. Reporting is the caller's.

    Returning rather than raising for an action the package does not act on: FR-011
    wants every one of them reported, and an exception here would turn "the peer is
    typing" into a refusal.
    """
    if isinstance(action, tl.DecryptedMessageActionNotifyLayer):
        # §7.2: "must always be updated immediately after receiving any packet
        # containing information of an upper layer". Raised only - §8.7 measured the
        # archived package assigning unconditionally, so a peer could walk its
        # announced layer back down and out of MTProto 2.0.
        chat.layer = framing.raise_remote_layer(chat.layer, action.layer)
        return Outcome(applied=True)

    if isinstance(action, tl.DecryptedMessageActionSetMessageTTL):
        # §5.1: "Store it and apply to subsequent messages; 0 disables." Stored and
        # transmitted, not enforced locally: §5 marks the moment a countdown starts
        # UNVERIFIED for every media type, and the spec's Assumptions take the
        # default of storing and transmitting rather than inventing one.
        if action.ttl_seconds < 0:
            await manager.close(chat.id, "negative message lifetime received")
            return Outcome(applied=False)
        chat.ttl = action.ttl_seconds
        return Outcome(applied=True)

    if isinstance(action, tl.DecryptedMessageActionDeleteMessages):
        manager._remove_history(chat.id, set(action.random_ids))
        with manager._atomic(chat):
            manager._rewrite_retained_as_deletes(chat, set(action.random_ids))
        return Outcome(applied=True)

    if isinstance(action, tl.DecryptedMessageActionFlushHistory):
        manager._history.pop(chat.id, None)
        with manager._atomic(chat):
            manager._rewrite_retained_as_deletes(
                chat,
                {
                    manager._retained_random_id(item)
                    for item in manager._storage.retained_out(chat.id)
                },
            )
        return Outcome(applied=True)

    if isinstance(action, tl.DecryptedMessageActionResend):
        # The manager authenticates and deduplicates this before acting immediately: it
        # "must always be interpreted immediately upon receipt in all cases". By the
        # time it reaches here it has been rewritten to Noop, so arriving here means
        # something replayed it - and "each decryptedMessageActionResend must only be
        # handled once".
        return Outcome(applied=False)

    if isinstance(action, REKEY):
        from . import rekey

        return await rekey.handle(manager, chat, action)

    # Everything else is the application's to act on: read receipts, deletions, a
    # history flush, a screenshot notice, a typing indicator, a Noop. The package
    # stores none of them, and reports all of them.
    return Outcome(applied=False)


# --- outbound (FR-010, §5) ----------------------------------------------------
# Thin by design: each is the constructor plus its section number, so the manager's
# public methods read as one line and §5's mapping stays visible in one place.


def set_message_ttl(seconds: int):
    """§5.1."""
    return tl.DecryptedMessageActionSetMessageTTL(ttl_seconds=seconds)


def read_messages(random_ids):
    """§5.2. "for TTL messages this is what starts the countdown on the sender's
    copy"."""
    return tl.DecryptedMessageActionReadMessages(random_ids=list(random_ids))


def delete_messages(random_ids):
    """§5.3, and the vehicle for §3.8's self-delete."""
    return tl.DecryptedMessageActionDeleteMessages(random_ids=list(random_ids))


def screenshot_messages(random_ids):
    """§5.4."""
    return tl.DecryptedMessageActionScreenshotMessages(random_ids=list(random_ids))


def flush_history():
    """§5.5."""
    return tl.DecryptedMessageActionFlushHistory()


def typing(action=None):
    """§5.8, reusing the cloud-chat ``SendMessageAction``."""
    return tl.DecryptedMessageActionTyping(action=action or tl.SendMessageTypingAction())


def notify_layer(layer: int):
    """§5.7."""
    return tl.DecryptedMessageActionNotifyLayer(layer=layer)


def resend(start_seq_no: int, end_seq_no: int):
    """§5.6. The bounds travel already transformed (§3.7)."""
    return tl.DecryptedMessageActionResend(start_seq_no=start_seq_no, end_seq_no=end_seq_no)
