"""An `async def` handler has to actually run.

Called synchronously, one returns a coroutine that nobody awaits: Python warns
"coroutine ... was never awaited" into stderr and the handler's whole body simply
does not happen. That is the worst shape a failure can take here - the application
sees a registered handler and a clean run.

Measured on a real chat between two accounts: an application that accepted
invitations inside an `async` `ChatRequested` handler left every one of them
`pending` for ever, with nothing in the answer to say so. And handlers are naturally
async, because the interesting ones accept a chat, answer a message or write a file.
"""

import asyncio

import pytest

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.events import ChatReady
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import Wire


@pytest.fixture
async def manager():
    wire = Wire()
    made = SecretChatManager(wire.a, storage=MemoryStorage())
    await made.start()
    yield made
    await made.stop()


async def test_an_async_handler_is_actually_run(manager):
    ran = asyncio.Event()

    async def handler(event):
        ran.set()

    manager.on("ChatReady", handler)
    manager._emit(ChatReady(1, 2, b"\x00" * 8))

    await asyncio.wait_for(ran.wait(), timeout=2)


async def test_a_sync_handler_still_runs_in_place(manager):
    """The existing shape keeps working, and keeps running synchronously - a test
    that only proved "eventually" would let a regression to scheduling pass."""
    seen = []
    manager.on("ChatReady", seen.append)

    manager._emit(ChatReady(1, 2, b"\x00" * 8))

    assert len(seen) == 1, "a plain handler was deferred or dropped"


async def test_one_failing_handler_does_not_stop_the_others(manager):
    """`_emit` runs inside the update path. A handler that raises there must not
    take down the decryption of every other chat."""
    reached = []

    def bad(event):
        raise RuntimeError("handler is broken")

    manager.on("ChatReady", bad)
    manager.on("ChatReady", reached.append)

    manager._emit(ChatReady(1, 2, b"\x00" * 8))

    assert reached, "a raising handler suppressed the one after it"


async def test_a_failing_async_handler_is_reported_rather_than_lost(manager, caplog):
    """A coroutine handed to `ensure_future` that raises reports "Task exception was
    never retrieved" at garbage-collection time, minutes later, naming nothing."""
    started = asyncio.Event()

    async def bad(event):
        started.set()
        raise RuntimeError("handler is broken")

    manager.on("ChatReady", bad)
    with caplog.at_level("ERROR"):
        manager._emit(ChatReady(1, 2, b"\x00" * 8))
        await asyncio.wait_for(started.wait(), timeout=2)
        await asyncio.sleep(0)

    assert any(
        "bad" in record.getMessage() for record in caplog.records
    ), "the failing handler was not named in the log"
