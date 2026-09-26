"""Validate sequence state in message order, retaining bounded gaps and envelopes."""

from __future__ import annotations

from typing import List, NamedTuple, Optional, Tuple

from telethon.extensions import BinaryReader

from . import framing
from .errors import MessageRejected, ResendUnsatisfiable
from .schema import secret_tl as tl

__all__ = ["accept", "answer_resend", "forget_acknowledged", "Accepted", "MAX_RESEND_COUNT"]
MAX_RESEND_COUNT = 1000


class Accepted(NamedTuple):
    ready: List[object]
    resend: Optional[Tuple[int, int]] = None
    duplicate: bool = False


def _abort(chat, reason):
    chat.close(reason)
    failure = MessageRejected(chat_id=chat.id, reason=reason)
    failure.fatal = True
    raise failure


def _queued(chat, storage):
    return storage.peek_in(chat.id)


def preflight(chat, wrapper, storage):
    """Validate before service-action side effects; return False for any replay."""
    expected_in = 1 if chat.is_outbound else 0
    expected_out = 0 if chat.is_outbound else 1
    if wrapper.in_seq_no < 0 or wrapper.out_seq_no < 0:
        _abort(chat, "sequence numbers must be nonnegative")
    if wrapper.in_seq_no % 2 != expected_in or wrapper.out_seq_no % 2 != expected_out:
        _abort(chat, "the sequence numbers do not have the parity this side must receive")
    raw_out = wrapper.out_seq_no // 2
    if raw_out < chat.in_seq_no:
        return False
    queued = _queued(chat, storage)
    if any(item["seq_no"] == raw_out for item in queued):
        return False
    _check_order(chat, wrapper)
    if raw_out - chat.in_seq_no > MAX_RESEND_COUNT or (
        raw_out > chat.in_seq_no and len(queued) >= MAX_RESEND_COUNT
    ):
        _abort(chat, "the incoming gap exceeds the supported recovery window")
    return True


def _check_order(chat, wrapper):
    peer_in = wrapper.in_seq_no // 2
    if peer_in < chat.peer_in_seq_no:
        _abort(chat, "the peer's echo of this side's counter went backwards")
    if peer_in > chat.out_seq_no:
        _abort(chat, "the peer claims to have seen a message this side has not sent")
    if wrapper.layer < chat.wrapper_layer:
        _abort(chat, "the peer encoded below the layer it had already used")


def pack(wrapper):
    return {
        "seq_no": wrapper.out_seq_no // 2,
        "body": bytes(wrapper).hex(),
        "file": getattr(wrapper, "_tsc_file", None),
    }


def unpack(record):
    with BinaryReader(bytes.fromhex(record["body"])) as reader:
        wrapper = tl.read_object(reader)
    wrapper._tsc_file = record.get("file")
    return wrapper


def accept(chat, wrapper, storage, envelope=None) -> Accepted:
    if not preflight(chat, wrapper, storage):
        return Accepted([], duplicate=True)
    attachment = getattr(envelope, "file", None)
    wrapper._tsc_file = bytes(attachment).hex() if attachment is not None else None
    raw_out = wrapper.out_seq_no // 2
    chat.layer = framing.raise_remote_layer(chat.layer, wrapper.layer)
    if raw_out > chat.in_seq_no:
        storage.queue_in(chat.id, pack(wrapper))
        if chat.gap_requested:
            return Accepted([])
        chat.gap_requested = True
        chat.gap_end = raw_out
        parity = wrapper.out_seq_no % 2
        return Accepted([], (2 * chat.in_seq_no + parity, wrapper.out_seq_no - 2))

    ready = []

    def advance(item):
        # An out-of-order packet was not a chronological predecessor when queued.
        _check_order(chat, item)
        chat.peer_in_seq_no = item.in_seq_no // 2
        chat.wrapper_layer = item.layer
        chat.in_seq_no += 1
        ready.append(item)

    advance(wrapper)
    waiting = []
    for record in storage.take_in(chat.id):
        if record["seq_no"] == chat.in_seq_no:
            advance(unpack(record))
        elif record["seq_no"] > chat.in_seq_no:
            waiting.append(record)
    for record in waiting:
        storage.queue_in(chat.id, record)
    resend = None
    if not waiting:
        chat.gap_requested = False
        chat.gap_end = None
    elif chat.gap_end is None or chat.in_seq_no >= chat.gap_end:
        # The first hole closed, but a distinct later hole remains.
        chat.gap_end = waiting[0]["seq_no"]
        parity = wrapper.out_seq_no % 2
        resend = (2 * chat.in_seq_no + parity, 2 * (chat.gap_end - 1) + parity)
        chat.gap_requested = True
    return Accepted(ready, resend)


def answer_resend(chat, storage, start_seq_no: int, end_seq_no: int) -> List[dict]:
    parity = 1 if chat.is_outbound else 0
    if (
        start_seq_no < 0
        or end_seq_no < start_seq_no
        or start_seq_no % 2 != parity
        or end_seq_no % 2 != parity
        or (end_seq_no - start_seq_no) // 2 + 1 > MAX_RESEND_COUNT
    ):
        _unsatisfiable(chat, (start_seq_no, end_seq_no), -1)
    held = {item["seq_no"]: item for item in storage.retained_out(chat.id)}
    wanted = range(start_seq_no, end_seq_no + 1, 2)
    if any(number not in held for number in wanted):
        _unsatisfiable(chat, (start_seq_no, end_seq_no), min(held) if held else -1)
    return [held[number] for number in wanted]


def _unsatisfiable(chat, span, retained_from):
    chat.close("a resend request could not be satisfied")
    failure = ResendUnsatisfiable(chat_id=chat.id, requested=span, retained_from=retained_from)
    failure.fatal = True
    raise failure


def forget_acknowledged(chat, storage, peer_in_seq_no_raw):
    if peer_in_seq_no_raw > 0:
        highest = 2 * (peer_in_seq_no_raw - 1) + (1 if chat.is_outbound else 0)
        storage.drop_out(chat.id, highest)
