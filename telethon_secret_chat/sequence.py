"""Sequence numbers, replay, gaps and resend - protocol-reference.md §3.4-§3.8.

PHASE 4 STATE, and it is stated rather than implied: this module currently accepts
and counts, and performs NONE of the §3.5/§3.6 validation. That is US3's work
(tasks T028-T033), whose tests must be watched failing against exactly this.

What it does do is own the counters, so that when the checks arrive they arrive in
one place. §3.4: the counters are "incremented strictly by 1 after any message
(service or not) is sent/received and processed".
"""

from __future__ import annotations

from typing import List

__all__ = ["accept"]


def accept(chat, wrapper) -> List:
    """Take one decoded wrapper and return the messages ready to deliver, in order.

    The list exists for §3.7: once a gap closes, a single arriving message releases
    everything held behind it, so the caller must be shaped for many even while this
    returns one.
    """
    chat.in_seq_no += 1
    return [wrapper]
