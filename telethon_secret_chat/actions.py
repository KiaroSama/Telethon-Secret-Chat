"""The thirteen service actions - protocol-reference.md §5.

PHASE 4 STATE: only `NotifyLayer` is acted on, because §7.3 makes it the one action
that must work before a conversation can be said to work at all. The other twelve
are reported to the application and not applied - US4's work (T034-T036), and its
tests are watched failing against exactly this.

The split that shapes the finished module, from data-model.md §4: Conversation
actions become events and may change stored chat state; Protocol and Rekey actions
are the package's own business, handled without asking the application, and are
still reported (FR-011).
"""

from __future__ import annotations

from typing import NamedTuple

from . import framing
from .schema import secret_tl as tl

__all__ = ["Outcome", "handle"]


class Outcome(NamedTuple):
    """Whether the package acted, as well as reported."""

    applied: bool


async def handle(manager, chat, action) -> Outcome:
    if isinstance(action, tl.DecryptedMessageActionNotifyLayer):
        # §7.2: "This remote layer value must always be updated immediately after
        # receiving any packet containing information of an upper layer." Raised
        # only - §8.7 measured the archived package assigning unconditionally, so a
        # peer could walk its announced layer back down.
        chat.layer = framing.raise_remote_layer(chat.layer, action.layer)
        return Outcome(applied=True)
    return Outcome(applied=False)
