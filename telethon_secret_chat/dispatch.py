"""Event registration and isolated callback dispatch for ``SecretChatManager``.

A callback belongs to the application, so its failure is the application's: it is
logged by name and never allowed to interrupt the protocol transition that emitted
it. Async callbacks run as owned tasks so ``stop()`` can cancel and drain them.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
from typing import Any, Callable

from .events import EVENT_TYPES
from .host import ManagerHost

__all__ = ["EventDispatch"]
log = logging.getLogger("telethon_secret_chat")


class EventDispatch(ManagerHost):
    """Mixed into the manager, which owns ``_handlers`` and ``_handler_tasks``."""

    def on(self, event: str, handler: Callable):
        """Register ``handler`` for an event name.

        Names: ChatRequested, ChatReady, ChatClosed (alias of ChatClosedEvent),
        MessageReceived, MessageAcknowledged, ServiceActionReceived, DecryptFailed,
        SendFailed. A sync handler runs inline; an async one is scheduled as a task
        and not awaited. A handler's exception is logged, never raised into the
        receive path.

        Raises: ValueError for an unknown name or a non-callable handler.
        """
        event = "ChatClosedEvent" if event == "ChatClosed" else event
        if event not in EVENT_TYPES or not callable(handler):
            raise ValueError("register a known secret-chat event and a callable handler")
        self._handlers.setdefault(event, []).append(handler)

    @staticmethod
    def _handler_name(handler):
        if inspect.isfunction(handler) or inspect.ismethod(handler):
            return handler.__name__
        return type(handler).__name__

    def _emit(self, event: Any):
        for handler in tuple(self._handlers.get(type(event).__name__, [])):
            try:
                result = handler(event)
            except Exception:
                log.error("secret-chat handler %s failed", self._handler_name(handler))
                continue
            if inspect.isawaitable(result) and self._stopping:
                self._discard_awaitable(handler, result)
                continue
            if inspect.isawaitable(result):
                task = asyncio.ensure_future(self._run_handler(handler, result))
                self._handler_tasks.add(task)
                if asyncio.isfuture(result):
                    self._handler_dependencies[task] = result

                def done(task, result=result, handler=handler):
                    self._handler_tasks.discard(task)
                    self._handler_dependencies.pop(task, None)
                    if inspect.iscoroutine(result):
                        result.close()  # Also close an awaitable cancelled before its wrapper starts.
                    elif asyncio.isfuture(result) and (task.cancelled() or not result.done()):
                        self._discard_awaitable(handler, result)

                task.add_done_callback(done)

    def _discard_awaitable(self, handler, result):
        if inspect.iscoroutine(result):
            result.close()
        elif asyncio.isfuture(result):
            # A Task can raise while handling cancellation. Retain it until its
            # completion and consume its failure rather than leaking it to asyncio.
            self._handler_tasks.add(result)
            self._draining_handlers.add(result)
            name = self._handler_name(handler)

            def retrieved(task):
                self._handler_tasks.discard(task)
                self._draining_handlers.discard(task)
                if not task.cancelled() and task.exception() is not None:
                    log.error("secret-chat handler %s failed during cleanup", name)

            result.add_done_callback(retrieved)
            result.cancel()

    async def _run_handler(self, handler, awaitable):
        try:
            await awaitable
        except Exception:
            # Never log exception text, traceback, arguments, or callable repr.
            log.error("secret-chat handler %s failed", self._handler_name(handler))

    async def _cancel_handler_tasks(self):
        tasks = [
            task
            for task in self._handler_tasks
            if task is not asyncio.current_task()
            and task not in self._stop_callers
            and self._handler_dependencies.get(task) not in self._stop_callers
        ]
        # Cancellation of a wrapper propagates to the Task it awaits. Both are
        # in the same drain, but only cancel the wrapper to avoid double requests.
        draining = set(tasks)
        draining.update(
            self._handler_dependencies[task]
            for task in tasks
            if task in self._handler_dependencies
        )
        self._draining_handlers.update(draining)
        try:
            for task in tasks:
                task.cancel()
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
        finally:
            self._draining_handlers.difference_update(draining)
