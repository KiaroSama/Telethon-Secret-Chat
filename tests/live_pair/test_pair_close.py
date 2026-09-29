"""Pair tier: how a close reaches the far side. Each case needs its own chat."""

import asyncio
import time

import pytest

from telethon_secret_chat import ChatClosed, ChatState

from ._pair import DEADLINE, POLL

pytestmark = pytest.mark.timeout(300)


async def test_a_close_reaches_the_other_side_on_its_next_update_sync(
    pair_chat, manager_a, manager_b, client_b
):
    """Telegram does not push a discard to the peer in real time: measured 2026-09-29,
    three runs, nothing within 20-60 s. It arrives with the next update difference,
    which Telethon fetches on reconnect, after a gap, or on ``client.catch_up()``."""
    chat_id, _, _ = pair_chat
    await manager_a.close(chat_id, reason="pair tier close check")
    until = time.monotonic() + DEADLINE
    while manager_b.status(chat_id).state is not ChatState.CLOSED:
        if time.monotonic() > until:
            pytest.fail("the accepting side never closed, even after fetching missed updates")
        await client_b.catch_up()
        await asyncio.sleep(POLL * 8)
    assert manager_b.recorder.first("ChatClosedEvent", lambda one: one.chat_id == chat_id)


async def test_sending_into_a_chat_the_peer_closed_closes_it_here(pair_chat, manager_a, manager_b):
    """Before any update sync, a send is the other way this side learns: the server's
    answer is a chat-ending rejection, and the chat closes here (outbox.CHAT_ENDING)."""
    chat_id, _, _ = pair_chat
    await manager_a.close(chat_id, reason="pair tier close check")
    with pytest.raises(ChatClosed):
        await manager_b.send_message(chat_id, "pair after the peer closed")
    assert manager_b.status(chat_id).state is ChatState.CLOSED
