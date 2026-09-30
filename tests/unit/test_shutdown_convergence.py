"""Shutdown failure/cancellation boundaries, including Python 3.14 diagnostics."""

import asyncio
import gc

import pytest

from telethon_secret_chat.errors import ManagerStopping
from telethon_secret_chat.events import ChatReady

from .helpers import ready_manager


@pytest.mark.parametrize("operation", ["start", "stop"])
@pytest.mark.parametrize("shape", ["coroutine", "task"])
async def test_handler_lifecycle_requests_during_cancellation_cleanup(operation, shape):
    manager, _ = ready_manager()
    await manager.start()
    entered, cleaned = asyncio.Event(), asyncio.Event()

    async def handler(event):
        try:
            entered.set()
            await asyncio.Event().wait()
        finally:
            if operation == "stop":
                await manager.stop()
            else:
                with pytest.raises(ManagerStopping):
                    await manager.start()
            cleaned.set()

    manager.on(
        "ChatReady",
        handler if shape == "coroutine" else lambda event: asyncio.create_task(handler(event)),
    )
    manager._emit(ChatReady(7, 9, 0))
    await asyncio.wait_for(entered.wait(), 2)
    handlers = list(manager._handler_tasks)
    stopping = asyncio.create_task(manager.stop())
    try:
        await asyncio.wait_for(stopping, 1)
        assert cleaned.is_set()
        assert not manager._running
    finally:
        # Also release a broken implementation; a regression must not hang pytest.
        for task in handlers:
            task.cancel()
        await asyncio.gather(*handlers, return_exceptions=True)
        if manager._stop_task is not None:
            await asyncio.gather(manager._stop_task, return_exceptions=True)


async def test_start_completes_a_previously_failed_shutdown_before_resubscribing(monkeypatch):
    manager, _ = ready_manager()
    await manager.start()
    save = manager._save

    def fail(chat):
        raise OSError("private storage details")

    monkeypatch.setattr(manager, "_save", fail)
    with pytest.raises(OSError):
        await manager.stop()
    monkeypatch.setattr(manager, "_save", save)
    await manager.start()
    try:
        assert manager._running and not manager._stopping
        await manager.send_message(7, "after recovered shutdown")
    finally:
        await manager.stop()


@pytest.mark.parametrize("cancelled_method", ["start", "stop"])
async def test_cancelled_lifecycle_waiter_never_reports_raw_failure(
    monkeypatch, caplog, cancelled_method
):
    manager, _ = ready_manager()
    await manager.start()
    entered, release = asyncio.Event(), asyncio.Event()
    save = manager._save

    async def hold_chat():
        async with manager._chat_lock(7):
            entered.set()
            await release.wait()

    def fail(chat):
        raise OSError("private lifecycle failure details")

    holder = asyncio.create_task(hold_chat())
    await asyncio.wait_for(entered.wait(), 2)
    monkeypatch.setattr(manager, "_save", fail)
    observer = asyncio.create_task(manager.stop())
    await asyncio.sleep(0)
    cancelled = asyncio.create_task(getattr(manager, cancelled_method)())
    await asyncio.sleep(0)
    cancelled.cancel()
    with pytest.raises(asyncio.CancelledError):
        await cancelled
    release.set()
    await holder
    with pytest.raises(OSError):
        await asyncio.wait_for(observer, 2)
    await asyncio.sleep(0)
    try:
        assert "shutdown failed: OSError" in caplog.text
        assert "private lifecycle failure details" not in caplog.text
        assert not manager._stop_task.cancelled()
    finally:
        monkeypatch.setattr(manager, "_save", save)
        await manager.stop()


@pytest.mark.parametrize("kind", ["completed_future", "cancelled_task"])
@pytest.mark.parametrize("stopping", [False, True])
async def test_discarded_handler_failures_are_retrieved_without_sensitive_logs(
    caplog, kind, stopping
):
    manager, _ = ready_manager()
    await manager.start()
    loop = asyncio.get_running_loop()
    errors = []
    previous = loop.get_exception_handler()
    loop.set_exception_handler(lambda loop, context: errors.append(context))
    entered = asyncio.Event()

    async def failing_cleanup():
        try:
            entered.set()
            await asyncio.Event().wait()
        finally:
            raise RuntimeError("sensitive handler cleanup details")

    if kind == "completed_future":
        future = loop.create_future()
        future.set_exception(RuntimeError("sensitive handler cleanup details"))
    else:
        future = asyncio.create_task(failing_cleanup())
        await asyncio.wait_for(entered.wait(), 2)
    results = [future]
    manager.on("ChatReady", lambda event: results.pop())
    manager._stopping = stopping
    try:
        manager._emit(ChatReady(7, 9, 0))
        if not stopping:
            # Cancel the wrapper before it ever awaits the returned Future/Task.
            wrappers = list(manager._handler_tasks)
            for wrapper in wrappers:
                wrapper.cancel()
            await asyncio.gather(*wrappers, return_exceptions=True)
            wrappers.clear()
            del wrapper
        del future
        for _ in range(4):
            await asyncio.sleep(0)
        gc.collect()
        assert not errors, "a discarded awaitable leaked an exception to the event loop"
        assert "sensitive handler cleanup details" not in caplog.text
    finally:
        loop.set_exception_handler(previous)
        await manager.stop()


async def test_returned_task_can_stop_without_its_wrapper_cancelling_it():
    manager, _ = ready_manager()
    await manager.start()
    finished = asyncio.Event()

    async def stop_from_handler():
        await manager.stop()
        finished.set()

    manager.on("ChatReady", lambda event: asyncio.create_task(stop_from_handler()))
    manager._emit(ChatReady(7, 9, 0))
    handlers = list(manager._handler_tasks)
    try:
        await asyncio.wait_for(finished.wait(), 2)
        assert not manager._running
    finally:
        for task in handlers:
            task.cancel()
        await asyncio.gather(*handlers, return_exceptions=True)
        await manager.stop()


async def test_discarded_task_cleanup_cannot_restart_the_stopping_manager():
    manager, _ = ready_manager()
    await manager.start()
    entered, finished = asyncio.Event(), asyncio.Event()

    async def cleanup():
        try:
            entered.set()
            await asyncio.Event().wait()
        finally:
            with pytest.raises(ManagerStopping):
                await manager.start()
            finished.set()

    task = asyncio.create_task(cleanup())
    await asyncio.wait_for(entered.wait(), 2)
    manager.on("ChatReady", lambda event: task)
    manager._stopping = True
    try:
        manager._emit(ChatReady(7, 9, 0))
        await asyncio.wait_for(finished.wait(), 2)
    finally:
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)
        await manager.stop()
