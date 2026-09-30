"""Pair tier: a delete for both sides reaches the other side (spec 007).

Deletes only the chat this case created, between the owner's two test accounts, with
the owner's permission (2026-09-30).
"""

import asyncio
import time

import pytest

from ._pair import DEADLINE, POLL

pytestmark = pytest.mark.timeout(300)


async def test_a_delete_for_both_sides_removes_the_chat_on_the_other_side(
    pair_chat, manager_a, manager_b, client_b
):
    chat_id, _, _ = pair_chat
    assert await manager_a.delete_secret_chat_both_sides(chat_id) is True
    assert chat_id not in [chat.id for chat in manager_a.list()]
    # Like a plain discard, it arrives with the next update difference (test_pair_close.py).
    until = time.monotonic() + DEADLINE
    while chat_id in [chat.id for chat in manager_b.list()]:
        if time.monotonic() > until:
            pytest.fail(
                "the other side never removed the chat, even after fetching missed updates"
            )
        await client_b.catch_up()
        await asyncio.sleep(POLL * 8)
    assert manager_b.recorder.first(
        "ChatClosedEvent", lambda one: one.chat_id == chat_id and one.history_deleted
    )
