# Architecture and contracts

How the package is put together, and the contracts a caller, a storage backend or an event
handler has to keep. The protocol itself is in [protocol-reference.md](protocol-reference.md)
(cited here as §N); the settled design decisions are the ADRs in [adr/](adr/). Citations
name a file and a symbol rather than a line, because lines move.

## 1. Module map

Dependencies point inward: the protocol modules at the top know nothing of the manager, the
manager composes them, and only the manager's parts (`manager.py`, `establishment.py`,
`outbox.py`) and `files.py` call Telethon's client.

| Module | Responsibility | Protocol |
|---|---|---|
| `dh.py` | validate Diffie-Hellman parameters and peer values before any exponentiation | §1.2 |
| `handshake.py` | the secret, the public value, the shared key, the fingerprint check; no network | §1.3-§1.6 |
| `crypto.py` | MTProto 2.0 frame: padding, `msg_key`, the vendored KDF, `encrypt_frame` / `decrypt_frame` with §2.7's checks | §2 |
| `framing.py` | `decryptedMessageLayer` wrap/unwrap, the layer arithmetic, the `seq_no` transform | §3.1-§3.4, §7 |
| `entities.py` | maps Telethon's message entities to the secret-chat schema, by the peer's layer | §7 |
| `sequence.py` | receive-side ordering: replay drop, gap queue, resend answers, acknowledgement | §3.4-§3.7 |
| `actions.py` | the thirteen service actions: outbound constructors and inbound handling | §5 |
| `rekey.py` | the four-message key exchange and the two-key window | §4 |
| `files.py` | one-time file keys, file encryption, the eight media kinds, `send` / `receive` / `forward` | §6 |
| `media.py` | media metadata checks, send sources (path, bytes, stream), `MediaReference` | §6 |
| `visualization.py` | `key_visualization`: the key picture as a colour grid and hex (standard library only) | §1.6 |
| `ogg_tags.py` | voice note or music: reads the Ogg comment block, never the audio | — |
| `chat.py` | the `SecretChat` entity, its state table and record validation | — |
| `storage/` | `StorageBackend` and the two shipped backends | — |
| `events.py` | the event dataclasses | — |
| `errors.py` | the `SecretChatError` family | — |
| `host.py` | `ManagerHost`: the state and steps the mixins share, declared for the type checker | — |
| `locking.py` | `ChatLocking` mixin: the per-chat lock, the outbound-order lock, `_atomic` | — |
| `dispatch.py` | `EventDispatch` mixin: handler registration and isolated callback dispatch | — |
| `outbox.py` | `RetainedOutbox` mixin: send, transmit, retry and rewrite retained outgoing records | §3.7, §3.8 |
| `establishment.py` | `Establishment` mixin: create, accept, the encryption updates, discard | §1.3-§1.6 |
| `receive.py` | `Receiving` mixin: update dispatch, decryption, sequence checks, the delivery mailbox | §2.7, §3 |
| `manager.py` | `SecretChatManager(ChatLocking, EventDispatch, RetainedOutbox, Establishment, Receiving)`: the public API and lifecycle | — |
| `schema/secret_tl.py` | generated from `schema/end-to-end.tl` by `tools/generate_schema.py`; never edited | — |

## 2. Locks and transactions

- **Per-chat lock.** `ChatLocking._chat_lock(chat_id)` is an `asyncio.Lock` per chat,
  re-entrant for the task that holds it (the owner is recorded in `_lock_owners`). Every
  public operation that changes a chat, and both update handlers, run under it through the
  `serialized` decorator. Chats do not block each other.
- **Atomic step.** `ChatLocking._atomic(chat)` opens one storage transaction, saves the
  whole chat record at the end of it, and on any exception restores the in-memory entity
  from a deep copy taken on entry. Memory and storage therefore move together or not at all.
- **No await inside a transaction.** Storage transactions are synchronous; the network call
  always happens after the commit (`outbox._send` commits the frame, the counter and the
  retained record, then calls `_transmit`). A crash between the two leaves a `pending`
  record that `retry_pending` sends with its original bytes and `random_id`.
- **Outbound order.** Public sends also take a per-chat outbound-order lock
  (`ChatLocking._outbound_lock`, the `ordered` decorator), always before the chat lock and
  never on the receive path. A file upload holds only that lock, so the chat keeps
  receiving while it uploads, and a text sent meanwhile still lands after the file.
- **Update dispatch.** Telethon runs update handlers as concurrent tasks;
  `receive._on_update` reaches the chat lock with no await before it, which keeps updates
  FIFO per chat.

## 3. The storage contract

`StorageBackend` (`storage/__init__.py`) is the only thing a custom backend implements. Every
method is atomic on its own; `transaction()` makes a group of them atomic together.

| Method | Promise |
|---|---|
| `transaction()` | synchronous context manager; all changes inside commit together or roll back, both the durable state and the backend's in-memory view. Nested scopes must work. Reads inside it see its own uncommitted writes. |
| `_commit(chat_id, record)` | make one complete record visible within the current transaction. Private-named but abstract: it is the method to implement; `save()` calls it with a deep copy. |
| `load(chat_id)` | the full record as a detached copy, or `None` |
| `delete(chat_id)` | remove the record AND both queues, atomically |
| `list()` | every chat id held |
| `queue_out(chat_id, item)` | upsert one retained outgoing record, keyed by `seq_no` |
| `drop_out(chat_id, up_to_seq)` | forget acknowledged outgoing records, inclusive |
| `retained_out(chat_id)` | detached retained records in `seq_no` order |
| `queue_in(chat_id, item)` | keep the first copy of a gap message; a duplicate `seq_no` must not grow storage |
| `take_in(chat_id)` | drain the gap queue in order |
| `requeue_in(chat_id, items)` | put still-waiting gap records back; the default queues them one by one |
| `peek_in(chat_id)` | inspect the gap queue without consuming it. The default is take-and-requeue inside a transaction, and it runs on EVERY received message (`sequence.preflight`), so a native backend should override it. |

Every read returns a detached copy; a caller mutating it must not change stored state.

**Record shape.** `SecretChat.to_record()` emits every name in `SecretChat._FIELDS` plus
`state`. Values are `None`, `bool`, `int`, `float`, `str`, `list`, `dict` and **`bytes`**
(the keys, the key hash), and some integers are **2048-bit** (`dh_prime`, `exchange_secret`,
`handshake["secret"]`): a SQL `INTEGER` cannot hold them. `FileStorage` writes JSON and tags
bytes as `{"__bytes__": "<hex>"}` (`FileStorage._encode` / `_decode`).

Retained outgoing records (`outbox._send`) hold `seq_no`, `body` (hex wrapper),
`frame` (hex ciphertext), `random_id`, `pending`, `method` (`message`, `service`, `file`),
`file` (hex `InputEncryptedFile`) and, after a rejected protocol message, `failures`. `outbox._transmit` uses `seq_no` plus **`body` equality**
as the record's identity, to tell whether a nested receive already replaced or removed it.
Gap records (`sequence.pack`) hold `seq_no`, `body` and `file`.

**The shipped backends.** `MemoryStorage` keeps one state dict and snapshots it on
entry to the outermost transaction. `FileStorage` extends it with a whole-state replace: every outermost
commit serializes the whole store to a temporary file in the same directory, fsyncs it and
`os.replace`s it over the store (plus a directory fsync on POSIX). It is one-process storage,
not a lock between processes, and it is not encrypted at rest. Opening it deletes
inactive, PID-owned `.secret-chat-store-<pid>-*.tmp` files. Cleanup preserves active,
unqueryable and legacy owners; a filename prefix alone is not evidence of a crash.
The same ownership registry protects decrypted-file writes in `files.save`.

## 4. The event contract

Eight events (`events.py`), each naming its chat and carrying a shape, never a key, a
plaintext or a wire object: `ChatRequested`, `ChatReady`, `ChatClosedEvent`,
`MessageReceived`, `MessageAcknowledged`, `ServiceActionReceived`, `DecryptFailed` and
`SendFailed` (Telegram rejected a sent message for good; it was withdrawn as a §3.8
self-delete).

- **Registration** (`dispatch.EventDispatch.on`): by class name; `"ChatClosed"` is an alias
  for `ChatClosedEvent`. An event with no handler is dropped.
- **Handlers before `start()`.** `start()` re-emits `ChatRequested` for pending requests and
  drains every chat's delivery mailbox immediately.
- **Delivery mailbox.** A received message is accepted into `chat.pending_deliveries` in the
  same transaction that advances the counters (`receive._on_encrypted_message`), and
  `_drain_deliveries` emits it and only then pops it in a new transaction. A crash in between
  delivers it again after restart: **at-least-once** for synchronous handlers. A stored item
  that no longer parses is dropped with a `DecryptFailed`, so it cannot hold the rest back.
- **Asynchronous handlers** are scheduled as owned tasks (`EventDispatch._emit`); the item is
  popped once the task is scheduled, so a crash can lose it. `stop()` cancels and drains
  those tasks.
- **Isolation.** A handler's exception is logged by the handler's name only and never
  interrupts the transition that emitted the event.
- **Order.** Per chat, `MessageReceived` follows conversation order (`seq_no`), and
  `MessageAcknowledged` for a received message is emitted before its delivery.

## 5. States

`chat._TRANSITIONS`:

| From | To |
|---|---|
| `requested` (this side asked) | `ready`, `closed` |
| `pending` (the peer asked) | `ready`, `closed` |
| `ready` | `rekeying`, `closed` |
| `rekeying` | `ready`, `closed` |
| `closed` | — (terminal) |

Sending works in `ready` and `rekeying` (`SecretChat.require_sendable`). Every state write
goes through the table (`SecretChat.transition_to`); only `close` and loading a record set
`state` directly.

**Terminal means terminal.** A closed chat is never reopened, and a restart is no loophole:
the record loads closed. The failures that close a chat are integrity failures, and going on
would mean trusting the counters that just failed.

**Key and fingerprint are one unit.** `SecretChat.adopt_key` is the only writer of both, and
a record stores them together, so no record pairs a key with another key's fingerprint.

**Closing.** `SecretChat.close` keeps the first reason and clears every key, the exchange
secret, the handshake and the mailbox; `manager._close_local` also deletes both queues and
re-saves the scrubbed record. The closed record stays as a **tombstone**, so a duplicate
request for the same id cannot revive the chat, until `forget(chat_id)` removes it.

## 6. Extension points

- **A storage backend.** Subclass `StorageBackend`, implement the abstract methods with the
  database's own transaction (never compensating writes), override `peek_in`, and store
  `bytes` and big integers losslessly.
- **History.** `read_history` is an in-memory convenience bounded by `history_limit`. An
  application that needs durable history keeps it from `MessageReceived`;
  [design/cutover-history.md](design/cutover-history.md) describes the media reference to
  store with it.
- **Media kinds.** `MEDIA_KINDS` and `CAPTIONLESS_KINDS` (`files.py`) are the vocabulary
  `send_file(kind=...)` accepts; a mismatch between kind and file type is refused, never
  converted.
- **Text parsing.** `send_message` uses the client's parse mode through the one private
  Telethon method the package calls (`TelegramClient._parse_message_text`, guarded by
  `tests/unit/test_telethon_canary.py`); passing `entities=` bypasses it.
