"""Durable orchestration of Telethon secret-chat state and ordered delivery.

Storage transactions are synchronous and never span a network await. Per-chat,
reentrant coroutine locks serialize protocol transitions without locking other
chats. Outgoing ciphertext and its sequence are committed together before the
RPC; uncertain sends remain retryable with their original identity and bytes.
The durable mailbox covers receive-to-dispatch. Applications own durable and
idempotent processing of callbacks; scheduling a callback is not its completion.
"""

from __future__ import annotations

import asyncio
import collections
import logging
import secrets
import time
from pathlib import Path
from typing import Callable, Dict, List, Optional


from . import actions as actions_module
from . import entities as entities_module, files, framing
from . import rekey as rekey_module
from .autosave import AutoSave
from .chat import ChatState, SecretChat
from .dispatch import EventDispatch
from .errors import ManagerStopping, StorageRequired, UnknownChat
from .events import ChatClosedEvent, ChatRequested
from .establishment import Establishment
from .locking import ChatLocking, ordered, serialized
from .outbox import RetainedOutbox
from .receive import Receiving
from .schema import secret_tl as tl
from .storage import FileStorage, StorageBackend

__all__ = ["SecretChatManager"]
log = logging.getLogger("telethon_secret_chat")


class SecretChatManager(ChatLocking, EventDispatch, RetainedOutbox, Establishment, Receiving):
    """Secret chats for one Telethon client, kept in one storage backend.

    Every protocol step of a chat runs under that chat's lock and inside one storage
    transaction, so a crash leaves either the whole step or none of it; different
    chats never wait on each other. Outgoing messages keep the order they were
    called in.

    Events reach handlers registered with ``on``; register them before ``start``.
    FileStorage defaults auto-save on beside its state file unless a saved preference
    overrides it; ``auto_save_folder`` selects an initial destination for other backends.
    Explicit MemoryStorage has no implicit archive path. ``start`` and ``stop`` are
    idempotent. API reference: ``help(SecretChatManager)``;
    design: ``docs/architecture.md``.
    """

    def __init__(
        self,
        client,
        storage: Optional[StorageBackend],
        *,
        history_limit: int = 1000,
        auto_save_folder: str | Path | None = None,
    ):
        if storage is None:
            raise StorageRequired()
        if type(history_limit) is not int or history_limit < 0:
            raise ValueError("history_limit must be a nonnegative integer")
        # ponytail: recent messages per chat, in memory; an application that needs
        # durable history keeps its own (README). Raise the limit, never unbound it.
        self._history_limit = history_limit
        self._client = client
        self._storage = storage
        self._chats: Dict[int, SecretChat] = {}
        self._history: Dict[int, collections.deque] = {}
        self._history_ids: Dict[int, set] = {}
        self._handlers: Dict[str, List[Callable]] = {}
        self._handler_tasks = set()
        self._handler_dependencies = {}
        self._locks = {}
        self._outbound_locks = {}
        self._lock_owners = {}
        self._inflight = set()
        self._creating = 0
        self._early_encryption = {}
        self._delivering = set()
        self._running = False
        self._stopping = False
        self._generation = 0
        self._stop_task: asyncio.Task[None] | None = None
        self._stop_callers = set()
        self._draining_handlers = set()
        self._subscription = self._on_update
        self._autosave = AutoSave(self)
        self._default_save_folder = (
            Path(auto_save_folder).absolute()
            if auto_save_folder is not None
            else (
                storage.path.parent / "saved-secret-chats"
                if isinstance(storage, FileStorage)
                else None
            )
        )

    async def start(self):
        """Load every stored chat, subscribe to updates, and resume durable work.

        Delivers what the mailbox still holds, retries pending sends and due Resend
        requests, and emits ``ChatRequested`` again for chats still awaiting
        ``accept``. Idempotent. Register handlers with ``on`` before calling it.

        Raises: StoreCorrupt if any stored record fails validation; nothing is then
        installed.
        """
        if asyncio.current_task() in self._draining_handlers:
            raise ManagerStopping()
        if self._stop_task is not None and not self._stop_task.done():
            await self._wait_stopped()
        elif self._stopping and self._running:
            # A failed save already removed the subscription. Finish that shutdown
            # before returning to RUNNING; otherwise start() would silently do nothing.
            await self.stop()
        if self._running:
            return
        loaded = {}
        for chat_id in self._storage.list():
            record = self._storage.load(chat_id)
            if record is not None:
                # All or nothing: one corrupt record installs no chat.
                loaded[chat_id] = SecretChat.from_record(record, stored_id=chat_id)
        self._chats = loaded
        # Before any mailbox is drained: those deliveries are saved too (spec 009).
        self._autosave.load()
        # Old uploads retain their old lock and generation. A restarted manager
        # must not wait for their network calls, nor accept their stale state.
        self._outbound_locks.clear()
        self._client.add_event_handler(self._subscription)
        self._running = True
        self._stopping = False
        # A snapshot: Telethon dispatches each update in its own task, so a request or a
        # create() can add a chat while this loop awaits recovery work.
        for chat in list(self._chats.values()):
            if chat.state is ChatState.CLOSED:
                self._scrub_closed_record(chat)
            elif chat.state in (ChatState.REQUESTED, ChatState.PENDING) and not chat.handshake:
                try:
                    await self.close(chat.id, "legacy pending handshake has no recoverable secret")
                except Exception:
                    log.warning("chat %s could not be closed at start", chat.id)
            elif chat.state is ChatState.PENDING:
                self._emit(ChatRequested(chat.id, chat.peer_user_id))
            elif chat.state in (ChatState.READY, ChatState.REKEYING):
                # Local before network, each on its own: a failed send must not keep
                # stored messages from being delivered.
                try:
                    await self._drain_deliveries(chat)
                except Exception:
                    log.warning("chat %s has undelivered mailbox items", chat.id)
                try:
                    await self.retry_pending(chat.id)
                    async with self._chat_lock(chat.id):
                        await self._request_due_resend(chat)
                except Exception:
                    log.warning("chat %s has durable work awaiting retry", chat.id)
        self._autosave.resume()
        log.info("secret-chat manager started with %s stored chats", len(self._chats))

    async def stop(self):
        """Unsubscribe, save every chat, and cancel handler tasks. Idempotent.

        Sends issued after this begins raise ManagerStopping. Pending sends stay in
        storage and are retried by the next ``start``.
        """
        caller = asyncio.current_task()
        if self._stopping and caller in self._draining_handlers:
            # The coordinator is already waiting for this handler's cancellation
            # cleanup. Waiting for the coordinator here would create a cycle.
            return
        self._stop_callers.add(caller)
        try:
            if self._stop_task is None or self._stop_task.done():
                if not self._running:
                    return
                self._stopping = True
                self._generation += 1
                self._stop_task = asyncio.create_task(self._finish_stop())
                self._stop_task.add_done_callback(self._report_stop_failure)
            # Cancelling one caller must not cancel the shared shutdown or another
            # handler waiting for it. Those handlers are excluded from the drain.
            await self._wait_stopped()
        finally:
            self._stop_callers.discard(caller)

    async def _wait_stopped(self) -> None:
        task = self._stop_task
        if task is None:
            return
        # wait() isolates caller cancellation without an exceptional proxy Future.
        # Python 3.14 shield() can report the raw exception after a cancelled wait,
        # even when our done callback has already retrieved/redacted that exception.
        await asyncio.wait((task,))
        task.result()  # A remaining caller still receives the original failure.

    @staticmethod
    def _report_stop_failure(task):
        # A caller may cancel its isolated wait; still retrieve the coordinator
        # exception without exposing storage error contents or a traceback.
        if not task.cancelled() and (failure := task.exception()) is not None:
            log.error("secret-chat shutdown failed: %s", type(failure).__name__)

    async def _finish_stop(self) -> None:
        self._client.remove_event_handler(self._subscription)
        # A cancelled download stays owed in pending.json; start() retries it.
        await self._autosave.cancel()
        self._stopping = True
        for chat in list(self._chats.values()):
            async with self._chat_lock(chat.id):
                self._save(chat)
        # After the sweep: a receive already queued on a lock may still emit and
        # schedule handlers while the sweep waits. Bounded, because a handler that
        # schedules more handlers must not keep stop() alive.
        for _ in range(3):
            if not self._handler_tasks:
                break
            await self._cancel_handler_tasks()
        self._running = False
        self._early_encryption.clear()
        log.info("secret-chat manager stopped")

    def _scrub_closed_record(self, chat):
        """Before PR #15 a closed chat kept its keys and queues on disk; drop them
        quietly - the chat already ended, so no event and no discard."""
        if not (
            chat.holds_material()
            or self._storage.retained_out(chat.id)
            or self._storage.peek_in(chat.id)
        ):
            return
        with self._atomic(chat):
            chat.scrub()
            self._storage.delete(chat.id)

    @serialized
    async def forget(self, chat_id):
        """Drop a closed chat's record; ``list()`` no longer shows it."""
        chat = self._require(chat_id)
        if chat.state is not ChatState.CLOSED:
            raise ValueError("only a closed chat can be forgotten")
        with self._storage.transaction():
            self._storage.delete(chat_id)
        self._chats.pop(chat_id, None)
        self._forget_history(chat_id)

    def _save(self, chat):
        self._storage.save(chat.to_record())

    def _close_local(self, chat, reason, history_deleted=False):
        with self._atomic(chat):
            # Scrub legacy closed records too, while preserving their first reason.
            reason = chat.closed_reason or reason
            chat.close(reason)
            self._storage.delete(chat.id)
        self._forget_history(chat.id)
        self._emit(ChatClosedEvent(chat.id, reason, history_deleted))

    def _remove_chat(self, chat):
        """Nothing of a closed chat stays here: record, queues, history (spec 007)."""
        with self._storage.transaction():
            self._storage.delete(chat.id)
        self._chats.pop(chat.id, None)
        self._forget_history(chat.id)

    def _deletable(self, chat_id):
        if self._stopping:
            raise ManagerStopping()
        return self._require(chat_id)

    @serialized
    async def delete_secret_chat(self, chat_id):
        """Delete a chat on this side only: an open chat ends (the peer sees it end and
        keeps its history), then its record, queues and history are removed here.
        Export first if the messages matter. Saved messages are not touched.

        Raises: UnknownChat; ManagerStopping.
        """
        chat = self._deletable(chat_id)
        if chat.state is not ChatState.CLOSED:
            self._close_local(chat, "deleted by this application")
            await self._discard_remote(chat.id)
        self._remove_chat(chat)

    @serialized
    async def delete_secret_chat_both_sides(self, chat_id) -> bool:
        """Delete a chat here and ask Telegram to erase the peer's history of it too.

        Best effort, like the official apps' "delete for both sides when possible": a
        chat that already ended on the server can no longer carry the request. This
        side is cleared either way. Saved messages are not touched.

        Returns: True when Telegram accepted the request, False when it could not.
        Raises: UnknownChat; ManagerStopping.
        """
        chat = self._deletable(chat_id)
        if chat.state is not ChatState.CLOSED:
            self._close_local(chat, "deleted for both sides", history_deleted=True)
        reached = await self._discard_remote(chat.id, delete_history=True)
        self._remove_chat(chat)
        return reached

    @serialized
    async def close(self, chat_id, reason="closed by this application"):
        """End a chat: the only closer. Every failure that ends a chat (a fatal
        ``SecretChatError``, a peer's discard, an aborted rekey) routes here."""
        chat = self._require(chat_id)
        if chat.state is ChatState.CLOSED:
            return
        self._close_local(chat, reason)
        await self._discard_remote(chat.id)

    def list(self):
        """Read-only snapshots (``ChatSnapshot``) of every chat, closed ones included."""
        return [chat.snapshot() for chat in self._chats.values()]

    def status(self, chat_id):
        """A read-only ``ChatSnapshot`` of one chat.

        Raises: UnknownChat.
        """
        return self._require(chat_id).snapshot()

    def _entity(self, chat_id):
        """The live chat, for this package and its tests; never handed to callers."""
        return self._require(chat_id)

    @ordered
    async def send_message(self, chat_id, text, entities=None, reply_to=None):
        """Send a text message.

        ``text`` is parsed with the client's default parse mode unless ``entities``
        is given; entities are mapped to the secret-chat schema and any the peer's
        layer cannot carry are dropped. Resolves when Telegram accepted the
        ciphertext.

        Returns: the message's ``random_id`` (what ``MessageAcknowledged`` names).
        Raises: UnknownChat, ChatNotReady, ChatClosed, ManagerStopping; SendPending
        when the message is stored but unconfirmed - it is retried, never send it
        again.
        """
        chat = self._sendable(chat_id)
        if entities is None and text:
            text, entities = await self._parse_text(text)
        entities = entities_module.to_secret(entities, layer=framing.outgoing_layer(chat.layer))
        await self._rekey_if_due(chat)
        random_id = secrets.randbits(63)
        await self._send(
            chat,
            tl.DecryptedMessage(
                random_id=random_id,
                ttl=chat.ttl,
                message=text,
                entities=entities,
                reply_to_random_id=reply_to,
            ),
        )
        return random_id

    @ordered
    async def rekey(self, chat_id):
        """Start a §4 key exchange now instead of waiting for the automatic trigger.

        Resolves once the RequestKey is sent; the switch completes as the peer
        answers. Raises: UnknownChat, ChatNotReady, ChatClosed.
        """
        await rekey_module.start(self, self._sendable(chat_id))

    async def _rekey_if_due(self, chat):
        if chat.state is ChatState.READY and rekey_module.should_rekey(chat, time.time()):
            await rekey_module.start(self, chat)

    async def send_file(
        self,
        chat_id,
        source,
        *,
        file_name=None,
        caption="",
        mime_type=None,
        kind=None,
        reply_to=None,
        duration=None,
        width=None,
        height=None,
        thumbnail=None,
        thumbnail_size=None,
        waveform=None,
        title=None,
        performer=None,
        sticker_alt=None,
    ):
        """Encrypt and upload a file, then send it as a document message.

        ``source`` is a path, bytes, or a seekable binary stream; bytes and streams need
        ``file_name``. The file is read and encrypted in pieces; ``kind`` (``MEDIA_KINDS``)
        picks the media shape, guessed from the file when omitted. The media metadata -
        ``duration`` (s), ``width``/``height`` (px), ``thumbnail`` (JPEG, WEBP for a sticker,
        under 200 KB) with ``thumbnail_size`` ``(w, h)`` each 1..320, ``waveform`` (voice note,
        5-bit samples, at most 63 bytes), ``title``/``performer`` (audio), ``sticker_alt`` -
        is what the peer's client shows before download; each applies only to its kinds.
        Messages sent meanwhile keep their call order.

        Returns: the message's ``random_id``.
        Raises: as ``send_message``, plus ValueError, before any upload, for an unusable
        source, metadata the kind cannot carry, or a size the peer's layer cannot encode.
        """
        metadata = {
            "duration": duration,
            "width": width,
            "height": height,
            "thumbnail": thumbnail,
            "thumbnail_size": thumbnail_size,
            "waveform": waveform,
            "title": title,
            "performer": performer,
            "sticker_alt": sticker_alt,
        }
        # Outbound order only: the upload runs without the chat lock (files.send).
        # Where a stream starts, so auto-save can copy the same bytes after the upload.
        start = source.tell() if hasattr(source, "read") else 0
        async with self._outbound_lock(chat_id):
            random_id = await files.send(
                self,
                self._sendable(chat_id),
                source,
                caption,
                mime_type,
                kind,
                reply_to,
                file_name=file_name,
                metadata=metadata,
            )
        name = file_name or (Path(source).name if isinstance(source, (str, Path)) else "file.bin")
        self._autosave.sent_file(chat_id, random_id, source, name, start)
        return random_id

    async def forward_file(self, chat_id, source, *, caption="", reply_to=None):
        """Send a received file into a chat again without downloading or uploading it.

        ``source`` is a ``MessageReceived`` with a file or a ``MediaReference``; the chat may
        be another one than it arrived in. The copy is encrypted with the file's ORIGINAL
        one-time key, so everyone who held that key can read it too (ADR 0006); send the
        file with ``send_file`` for a fresh key.

        Returns: the message's ``random_id``.
        Raises: as ``send_message``; MessageRejected for a source without a file;
        ValueError for a file over 2000 MB when the peer is below layer 143.
        """
        async with self._outbound_lock(chat_id):
            random_id = await files.forward(
                self, self._sendable(chat_id), source, caption, reply_to
            )
        self._autosave.forwarded(chat_id, random_id, source)
        return random_id

    # --- auto-save (spec 009) ---------------------------------------------------------

    @property
    def auto_save_secret_chats(self) -> Optional[str]:
        """The auto-save folder, or None when auto-save is off."""
        return str(self._autosave.folder) if self._autosave.folder is not None else None

    async def start_auto_save_secret_chats(self, folder) -> None:
        """Save every message and file of every secret chat, from now on, into ``folder``.

        Self-destructing messages included; files are downloaded and decrypted at once,
        whatever their size. Stays on across restarts (kept in the storage) until
        ``stop_auto_save_secret_chats``. Saved data outlives message and chat deletes. The
        folder holds PLAINTEXT: protect it like the store.
        """
        self._autosave.enable(folder)

    async def stop_auto_save_secret_chats(self) -> None:
        """Save nothing more. What was saved stays; ``delete_saved_messages`` removes it."""
        await self._autosave.settle()
        self._autosave.disable()

    def read_saved_messages(self, chat_id) -> List[dict]:
        """The saved records of one chat, in order (also for a deleted chat).

        Each is a dict: ``type`` (``message`` or ``service``), ``id``, ``date`` (when this
        side saved it), ``out``, and for a message ``text``, ``entities``, ``ttl``,
        ``reply_to``, ``media`` (the protocol media without its key) and ``file`` (the
        saved file's path, once it is on disk); for a service record, ``action``.
        """
        return self._autosave.read(chat_id)

    def delete_saved_messages(self, chat_id) -> None:
        """Remove one chat's saved messages and files. Ask the user first."""
        self._autosave.delete(chat_id)

    async def export_saved_messages(
        self,
        chat_id,
        path,
        *,
        self_user,
        peer_user,
        format="html",
        media_types=None,
        size_limit=8 * 1024 * 1024,
        date_from=0,
        date_till=0,
        force_sub_path=False,
    ):
        """Export the retained saved copy as Desktop HTML, JSON or both, without RPCs.

        Supply participant Telethon Users; deleted chats remain exportable. Media groups:
        photo, video, voice_message, video_message, sticker, gif, file (default photos).
        Date bounds are Unix timestamps; 0 is open. Source data is never removed.
        Returns the shared ExportResult. Raises ValueError for invalid options, OSError
        for an export failure. Output contains plaintext; protect it like the saved copy.
        """
        from .saved_export import export

        return export(
            self.read_saved_messages(chat_id),
            path,
            self_user=self_user,
            peer_user=peer_user,
            source_folder=self._autosave.saved_in,
            format=format,
            media_types=media_types,
            size_limit=size_limit,
            date_from=date_from,
            date_till=date_till,
            force_sub_path=force_sub_path,
        )

    async def save_file(self, message, path) -> Path:
        """Download, verify and decrypt a received file to ``path``.

        ``message`` is a ``MessageReceived`` or a ``MediaReference`` (for a file received
        before a restart). The key's fingerprint is checked before a byte is written; a
        partial file is never left behind.

        Returns: the written path.
        Raises: MessageRejected for a message without a file, a fingerprint mismatch
        or invalid metadata.
        """
        return await files.receive(self, message, path)

    @ordered
    async def set_ttl(self, chat_id, seconds):
        """Set the self-destruct timer for later messages (0 disables).

        Raises: ValueError for a value outside 0..2**31-1; as ``send_message``.
        """
        chat = self._sendable(chat_id)
        if type(seconds) is not int or not 0 <= seconds < 2**31:
            raise ValueError("TTL must be a nonnegative signed 32-bit integer")
        await self._send_action(
            chat,
            actions_module.set_message_ttl(seconds),
            after_prepare=lambda: setattr(chat, "ttl", seconds),
        )

    @ordered
    async def mark_read(self, chat_id, random_ids):
        """Tell the peer these messages were read. Raises: as ``send_message``."""
        await self._send_action(self._sendable(chat_id), actions_module.read_messages(random_ids))

    @ordered
    async def delete_messages(self, chat_id, random_ids):
        """Delete messages on both sides.

        Local copies (history and any retained unsent content) are removed first,
        even if the network then fails. Raises: as ``send_message``.
        """
        chat = self._sendable(chat_id)
        ids = list(random_ids)
        # Remove content BEFORE retrying an uncertain send; privacy cannot wait
        # for the network to succeed. The retained self-delete preserves its slot.
        self._forget_locally(chat, set(ids))
        await self._send_action(chat, actions_module.delete_messages(ids))

    @ordered
    async def screenshot(self, chat_id, random_ids):
        """Tell the peer a screenshot was taken of these messages.

        Raises: as ``send_message``.
        """
        await self._send_action(
            self._sendable(chat_id), actions_module.screenshot_messages(random_ids)
        )

    @ordered
    async def flush_history(self, chat_id):
        """Clear the whole history on both sides.

        Raises: as ``send_message``.
        """
        chat = self._sendable(chat_id)
        self._forget_everything_locally(chat)
        await self._send_action(chat, actions_module.flush_history())

    @ordered
    async def set_typing(self, chat_id, action=None):
        """Send a typing (or other activity) indicator; ``None`` means typing.

        Raises: as ``send_message``.
        """
        await self._send_action(self._sendable(chat_id), actions_module.typing(action))

    def _sendable(self, chat_id):
        chat = self._require(chat_id)
        chat.require_sendable()
        return chat

    def read_history(self, chat_id, limit=50):
        """The most recent received messages of a chat, oldest first.

        In memory only: since this process started, at most ``history_limit`` per
        chat (the constructor argument). ``limit`` 0 returns nothing.

        Raises: UnknownChat; ValueError for a limit that is not a nonnegative int.
        """
        self._require(chat_id)
        if type(limit) is not int or limit < 0:
            raise ValueError("history limit must be a nonnegative integer")
        return list(self._history.get(chat_id, ()))[-limit:] if limit else []

    async def _parse_text(self, text):
        parse = getattr(self._client, "_parse_message_text", None)
        if parse is None:
            return text, None
        try:
            # `()` is Telethon's "the client's own parse mode"; `None` means "do not
            # parse", which silently sent every `**bold**` as literal asterisks.
            return await parse(text, ())
        except Exception:
            log.debug("text parser failed; using caller text without generated entities")
            return text, None

    def _require(self, chat_id):
        chat = self._chats.get(chat_id)
        if chat is None:
            raise UnknownChat(chat_id=chat_id)
        return chat
