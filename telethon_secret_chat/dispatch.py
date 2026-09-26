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

__all__ = ["EventDispatch"]
log = logging.getLogger("telethon_secret_chat")


class EventDispatch:
    """Mixed into the manager, which owns ``_handlers`` and ``_handler_tasks``."""

    def on(self, event: str, handler: Callable):
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
            if inspect.isawaitable(result):
                task = asyncio.ensure_future(self._run_handler(handler, result))
                self._handler_tasks.add(task)

                def done(task, result=result):
                    self._handler_tasks.discard(task)
                    if inspect.iscoroutine(result):
                        result.close()  # Also close an awaitable cancelled before its wrapper starts.
                    elif asyncio.isfuture(result) and not result.done():
                        result.cancel()

                task.add_done_callback(done)

    async def _run_handler(self, handler, awaitable):
        try:
            await awaitable
        except Exception:
            # Never log exception text, traceback, arguments, or callable repr.
            log.error("secret-chat handler %s failed", self._handler_name(handler))

    async def _cancel_handler_tasks(self):
        tasks = [task for task in self._handler_tasks if task is not asyncio.current_task()]
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
