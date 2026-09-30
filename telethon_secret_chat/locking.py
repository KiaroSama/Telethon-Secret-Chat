"""Per-chat locks and the storage transaction every protocol step runs in.

Storage transactions are synchronous and never span a network await. The
per-chat lock is reentrant by task, so a step that sends while it holds the
lock can call other serialized steps of the same chat. Mixed into
``SecretChatManager``, which owns ``_locks``, ``_lock_owners`` and ``_storage``.
"""

from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager, contextmanager
from copy import deepcopy
from functools import wraps
from .host import ManagerHost
from .errors import ManagerStopping

__all__ = ["ChatLocking", "ordered", "serialized"]


def serialized(method):
    @wraps(method)
    async def run(self, subject, *args, **kwargs):
        chat_id = (
            subject
            if isinstance(subject, int)
            else getattr(subject, "chat_id", getattr(subject, "id", None))
        )
        async with self._chat_lock(chat_id):
            return await method(self, subject, *args, **kwargs)

    return run


def ordered(method):
    """A public send: outbound order first, then the chat lock.

    Lock order is always outbound -> chat, never the reverse, and the receive path
    takes only the chat lock - so an upload holding the outbound lock never makes a
    chat deaf to incoming messages, acknowledgements or rekey steps.
    """
    locked = serialized(method)

    @wraps(method)
    async def run(self, chat_id, *args, **kwargs):
        async with self._outbound_lock(chat_id):
            return await locked(self, chat_id, *args, **kwargs)

    return run


class ChatLocking(ManagerHost):
    @asynccontextmanager
    async def _outbound_lock(self, chat_id):
        """Keeps this application's own sends in call order. Not reentrant."""
        generation = self._generation
        self._check_generation(generation)
        async with self._outbound_locks.setdefault(chat_id, asyncio.Lock()):
            self._check_generation(generation)
            yield

    def _check_generation(self, generation):
        if self._stopping or generation != self._generation:
            raise ManagerStopping()

    def _check_current(self, chat):
        # A detached entity must never overwrite the record installed by start().
        if self._chats.get(chat.id) is not chat:
            raise ManagerStopping()

    @asynccontextmanager
    async def _chat_lock(self, chat_id):
        task = asyncio.current_task()
        if self._lock_owners.get(chat_id) is task:
            yield
            return
        lock = self._locks.setdefault(chat_id, asyncio.Lock())
        async with lock:
            self._lock_owners[chat_id] = task
            try:
                yield
            finally:
                self._lock_owners.pop(chat_id, None)

    @contextmanager
    def _atomic(self, chat):
        self._check_current(chat)
        previous = deepcopy(vars(chat))
        try:
            with self._storage.transaction():
                yield
                self._save(chat)
        except BaseException:
            vars(chat).clear()
            vars(chat).update(previous)
            raise
