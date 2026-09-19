"""Sequence numbers, replay, gaps and resend - protocol-reference.md §3.4-§3.8.

The eight checks §3.5 and §3.6 make mandatory, of which §8.3 measured the archived
package performing **none** - the whole area sat under a literal ``# TODO add
checks``. Each documented rule here ends, in the source, with "the client is
required to immediately abort the secret chat", so a failure closes the chat rather
than dropping a message. There are exactly two exceptions, both documented: a replay
is dropped, and a gap is held.

The order inside ``accept`` is the order the reference states the rules in, and one
point of it matters in particular: §3.6's checks run BEFORE a message is queued for
a gap. A message held behind a hole is a message whose fields were already
validated; queueing one that violates §3.6 would defer an abort the documentation
calls immediate until the hole happened to close.
"""

from __future__ import annotations

from typing import List, NamedTuple, Optional, Tuple

from telethon.extensions import BinaryReader

from . import framing
from .errors import MessageRejected, ResendUnsatisfiable
from .schema import secret_tl as tl

__all__ = ["accept", "answer_resend", "forget_acknowledged", "Accepted", "MAX_RESEND_COUNT"]

# §3.7: TDLib's ``static constexpr int32 MAX_RESEND_COUNT = 1000;`` and the "Can't
# resend too many messages" refusal above it. The spec's Assumptions: "adopt the
# reference implementation's span cap rather than inventing a number".
MAX_RESEND_COUNT = 1000


class Accepted(NamedTuple):
    """What one arriving message produced.

    ``ready`` is empty for a replay (dropped) and for a message held behind a gap;
    it holds several when a hole closes and releases what was queued behind it.
    ``resend`` is the transformed span to ask for, or ``None`` - one request per
    hole, never one per out-of-order message.
    """

    ready: List[object]
    resend: Optional[Tuple[int, int]] = None


def _abort(chat, reason: str) -> None:
    """Every §3.4/§3.6 violation ends here. "the client is required to immediately
    abort the secret chat" - so the chat is closed BEFORE the error is raised, and a
    caller that swallows the exception still finds a closed chat."""
    chat.close(reason)
    raise MessageRejected(chat_id=chat.id, reason=reason)


def accept(chat, wrapper, storage) -> Accepted:
    """One decoded wrapper in; the messages ready to deliver, in order, out.

    §3.4 parity, then §3.6's echo, then §3.5's continuity, then the gap queue.
    """
    # --- §3.4, parity ---------------------------------------------------------
    # The peer's transform is the mirror of ours: if we originated, the peer is the
    # recipient, so its in_seq_no carries x=1 and its out_seq_no carries x=0.
    expected_in = 1 if chat.is_outbound else 0
    expected_out = 0 if chat.is_outbound else 1
    if wrapper.in_seq_no % 2 != expected_in or wrapper.out_seq_no % 2 != expected_out:
        # "This is done to prevent a possible attacker from mirroring the messages."
        _abort(chat, "the sequence numbers do not have the parity this side must receive")

    peer_out = wrapper.out_seq_no // 2
    peer_in = wrapper.in_seq_no // 2

    # --- §3.6, our own counter coming back ------------------------------------
    if peer_in < chat.peer_in_seq_no:
        _abort(chat, "the peer's echo of this side's counter went backwards")
    # "if D is the out_seq_no of last message we sent, the received in_seq_no should
    # not be greater than D + 1". We have sent `out_seq_no` messages, so the last
    # one's raw counter is `out_seq_no - 1` and the ceiling is `out_seq_no` itself.
    if peer_in > chat.out_seq_no:
        _abort(chat, "the peer claims to have seen a message this side has not sent")
    # TDLib's third condition, with no documentation counterpart (§3.6, marked
    # UNVERIFIED there): a peer may not walk its announced layer backwards, because
    # a peer that can would walk it below 73 and out of MTProto 2.0.
    if wrapper.layer < chat.layer:
        _abort(chat, "the peer announced a lower layer than it had already claimed")

    # §7.2: "must always be updated immediately after receiving any packet
    # containing information of an upper layer" - immediately, so here rather than
    # at delivery: a message held behind a gap has still been RECEIVED.
    chat.layer = framing.raise_remote_layer(chat.layer, wrapper.layer)
    chat.peer_in_seq_no = peer_in

    # --- §3.5, the peer's counter ---------------------------------------------
    if peer_out < chat.in_seq_no:
        # "the local client must drop the message (repeated message). The client
        # should not check the contents of the message because the original message
        # could have been deleted." Dropped - a replay does not end a chat.
        return Accepted(ready=[])

    if peer_out > chat.in_seq_no:
        # A gap. "Note that in_seq_no is not increased upon receipt of such a
        # message; it is advanced only after all preceding gaps are filled."
        storage.queue_in(chat.id, {"seq_no": peer_out, "body": bytes(wrapper).hex()})
        resend = None
        if not chat.gap_requested:
            # §3.7: "adding 2 to the out_seq_no of the last message before the hole"
            # and "subtracting 2 from the out_seq_no of the received message". The
            # span travels transformed, so the arithmetic stays on the wire values.
            last_before_hole = 2 * (chat.in_seq_no - 1) + wrapper.out_seq_no % 2
            resend = (last_before_hole + 2, wrapper.out_seq_no - 2)
            chat.gap_requested = True
        return Accepted(ready=[], resend=resend)

    # --- in sequence: deliver it, then drain whatever was waiting on it --------
    chat.in_seq_no += 1
    ready = [wrapper]

    # §3.7: "interpret recovered messages in seq_no order first, then drain the
    # queue in seq_no order". `take_in` returns them sorted and empties the queue.
    still_waiting = []
    for item in storage.take_in(chat.id):
        if item["seq_no"] == chat.in_seq_no:
            with BinaryReader(bytes.fromhex(item["body"])) as reader:
                ready.append(tl.read_object(reader))
            chat.in_seq_no += 1
        elif item["seq_no"] > chat.in_seq_no:
            still_waiting.append(item)
        # Anything below is a duplicate of something already delivered: dropped.
    for item in still_waiting:
        storage.queue_in(chat.id, item)
    if not still_waiting:
        chat.gap_requested = False

    return Accepted(ready=ready)


def answer_resend(chat, storage, start_seq_no: int, end_seq_no: int) -> List[dict]:
    """§3.7: re-send a span the peer is missing, or end the chat.

    ``start_seq_no`` and ``end_seq_no`` arrive TRANSFORMED and the retention queue
    holds the same transformed values, so nothing is converted here - which is the
    point. §8.3 measured the archived package answering a resend by composing a NEW
    message with a NEW ``out_seq_no``, which cannot fill the peer's hole however
    correct its contents are.
    """
    if end_seq_no < start_seq_no:
        _unsatisfiable(chat, (start_seq_no, end_seq_no), -1)
    if (end_seq_no - start_seq_no) // 2 + 1 > MAX_RESEND_COUNT:
        # TDLib: "Can't resend too many messages".
        _unsatisfiable(chat, (start_seq_no, end_seq_no), -1)

    held = {m["seq_no"]: m for m in storage.retained_out(chat.id)}
    wanted = list(range(start_seq_no, end_seq_no + 1, 2))
    if [seq for seq in wanted if seq not in held]:
        _unsatisfiable(chat, (start_seq_no, end_seq_no), min(held) if held else -1)
    return [held[seq] for seq in wanted]


def _unsatisfiable(chat, span, retained_from: int) -> None:
    """The protocol's answer to a resend it cannot serve is to end the chat, so the
    chat is closed before the error leaves.

    The span is safe to name - sequence numbers are positions, not content - and the
    retained BODIES are plaintext, so none of them appears.
    """
    chat.close("a resend request could not be satisfied")
    raise ResendUnsatisfiable(chat_id=chat.id, requested=span, retained_from=retained_from)


def forget_acknowledged(chat, storage, peer_in_seq_no_raw: int) -> None:
    """Drop retained messages the peer's echo says it has processed.

    §3.6's ``in_seq_no`` is the peer's count of messages taken in, so everything
    strictly below it can no longer be the subject of a resend. Its own function
    because §3.7's retention and §3.6's echo are otherwise two numbers that look
    unrelated.
    """
    if peer_in_seq_no_raw <= 0:
        return
    highest_acknowledged = 2 * (peer_in_seq_no_raw - 1) + (1 if chat.is_outbound else 0)
    storage.drop_out(chat.id, highest_acknowledged)
