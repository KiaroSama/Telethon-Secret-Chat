"""The state and steps the manager's parts share, declared once for the type checker.

``SecretChatManager`` is assembled from mixins (locking, dispatch, outbox,
establishment, receive), and each reaches state or steps another part owns. This
class writes that shared surface down in one place: annotations only, so it adds
nothing at run time, and the manager's own ``__init__`` and methods are the
implementations.
"""

from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, Any, Awaitable, Callable, ContextManager, Deque, Dict, List, Set

if TYPE_CHECKING:
    from .chat import SecretChat
    from .storage import StorageBackend

__all__ = ["ManagerHost"]


class ManagerHost:
    # State, set in SecretChatManager.__init__.
    _client: Any
    _storage: StorageBackend
    _chats: Dict[int, SecretChat]
    _history: Dict[int, Deque[Any]]
    _history_ids: Dict[int, Set[int]]
    _history_limit: int
    _handlers: Dict[str, List[Callable[..., Any]]]
    _handler_tasks: Set[asyncio.Future[Any]]
    _locks: Dict[int, asyncio.Lock]
    _outbound_locks: Dict[int, asyncio.Lock]
    _lock_owners: Dict[int, Any]
    _inflight: Set[Any]
    _creating: int
    _early_encryption: Dict[int, Any]
    _delivering: Set[int]
    _stopping: bool
    _generation: int
    _stop_callers: Set[Any]
    _draining_handlers: Set[asyncio.Future[Any]]
    _handler_dependencies: Dict[asyncio.Future[Any], asyncio.Future[Any]]

    # Steps one part calls on another.
    _check_generation: Callable[[int], None]
    _check_current: Callable[[SecretChat], None]
    _atomic: Callable[[SecretChat], ContextManager[None]]
    _save: Callable[[SecretChat], None]
    _emit: Callable[[Any], None]
    _require: Callable[[int], SecretChat]
    _sendable: Callable[[int], SecretChat]
    _close_local: Callable[..., None]
    _remove_chat: Callable[[SecretChat], None]
    _forget_history: Callable[[int], None]
    _remove_history: Callable[[int, Any], None]
    _retained_random_id: Callable[[Dict[str, Any]], int]
    close: Callable[..., Awaitable[None]]
    retry_pending: Callable[[int], Awaitable[None]]
    _send_action: Callable[..., Awaitable[None]]
    _rekey_if_due: Callable[[SecretChat], Awaitable[None]]
    _request_due_resend: Callable[[SecretChat], Awaitable[None]]
    _on_encryption: Callable[[Any], Awaitable[None]]
