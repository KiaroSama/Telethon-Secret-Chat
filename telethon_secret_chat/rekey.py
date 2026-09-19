"""Perfect forward secrecy - protocol-reference.md §4.

PHASE 6 STATE: the four rekey actions are recognised and reported, and none of the
exchange is driven yet. That is US6's work (T044-T046), whose tests are watched
failing against exactly this.

The one thing already correct: an inbound `AbortKey` clears the local exchange.
§8.4 measured the archived package letting it fall through to the application, so a
peer's abort never cleared `rekeying` and the chat could deadlock half-open.
"""

from __future__ import annotations

from .schema import secret_tl as tl

__all__ = ["handle"]


async def handle(manager, chat, action):
    from .actions import Outcome

    if isinstance(action, tl.DecryptedMessageActionAbortKey):
        # §4.6: "Receiving it must clear the local exchange state."
        if chat.exchange_id == action.exchange_id:
            chat.exchange_id = None
            chat.pending_key = None
        return Outcome(applied=True)
    return Outcome(applied=False)
