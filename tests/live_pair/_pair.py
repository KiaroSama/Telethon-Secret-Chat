"""Helpers for the pair tier: a started manager that records, and bounded waits.

Both ends of every chat here are this package, on two of the owner's accounts, over the
real Telegram server. That makes this tier evidence about the real server - its answers,
its file storage, its delivery - and NOT interop evidence: two copies of the same code
agree with each other whatever they get wrong (Principle I). Only ``tests/interop``,
with an official client on the far end, carries that claim.
"""

from __future__ import annotations

import asyncio
import time
from typing import List

import pytest

from interop._live import Recorder

from telethon_secret_chat import SecretChatManager

# No person is involved, so a step that takes longer than this is a failure, not a wait.
DEADLINE = 60.0
POLL = 0.25

EVENTS = (
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
    "SendFailed",
)


async def started_manager(client, storage, *, accept: bool) -> SecretChatManager:
    """A started manager whose events land in ``manager.recorder``.

    ``accept`` makes it accept every incoming request, which is what the accepting
    account does in place of a person. A failed accept is recorded, never swallowed.
    """
    manager = SecretChatManager(client, storage)
    recorder = Recorder()
    manager.recorder = recorder
    manager.accept_failures = []
    for name in EVENTS:
        manager.on(name, recorder.events.append)
    if accept:
        pending = set()

        def on_request(event):
            async def run():
                try:
                    await manager.accept(event.chat_id)
                except Exception as failure:  # reported by the waiting test
                    manager.accept_failures.append(type(failure).__name__)

            task = asyncio.ensure_future(run())
            pending.add(task)
            task.add_done_callback(pending.discard)

        manager.on("ChatRequested", on_request)
    await manager.start()
    return manager


async def await_texts(manager, chat_id: int, expected: List[str]) -> List[str]:
    """The texts received in ``chat_id``, once ``expected`` all arrived, in arrival order."""
    until = time.monotonic() + DEADLINE
    while time.monotonic() < until:
        got = [
            event.text
            for event in manager.recorder.of("MessageReceived")
            if event.chat_id == chat_id and event.text in expected
        ]
        if len(got) >= len(expected):
            return got
        await asyncio.sleep(POLL)
    failures = [event.reason for event in manager.recorder.of("DecryptFailed")]
    pytest.fail(
        f"waited {DEADLINE:.0f}s for {len(expected)} messages, got {len(got)}; "
        f"refused on arrival: {failures[:3] or 'none'}"
    )
