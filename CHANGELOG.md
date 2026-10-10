# Changelog

Every change to the public API gets a line here in the same commit. The consumer pins this
package by commit and reads this file before moving its pin. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) with the pre-1.0 rule that a minor bump may break.

## [Unreleased]

### Added

- File-backed `SecretChatManager` automatically enables saved copies beside the state file
  when no preference exists; `auto_save_folder=` supplies a custom initial destination.
  Durable chat/queue recovery continues healthy chats after restart. Explicit OFF and
  previously chosen archive locations persist; the transient opt-in remains payload-free.
- Opt-in `TransientSecretChatManager`, `ProtectedFileStorage` and validated `TransientLimits`:
  ordinary and service payloads remain bounded RAM-only, using the caller's existing client.
  Protected keys/counters survive; stop/reconstruction permanently suspends known chats without
  automatic discard or claimed lossless replay. Includes explicit acceptance control, bounded
  async callbacks and memory-only media receipt. See [limitations](docs/transient-mode.md).
- `export_saved_messages`: offline Desktop-style HTML, JSON or both from retained saved
  records, with media, size, date and destination options. The shared exporter and assets
  are copied unchanged; Pillow handles images and the optional Windows `thumbs` extra
  enables the tested Desktop JPEG/ICC path. No new login or server takeout is needed.
- Auto-save: `start_auto_save_secret_chats(folder)` / `stop_auto_save_secret_chats()` save every
  message of every secret chat (self-destructing ones included) and download every file at once;
  `read_saved_messages` / `delete_saved_messages`. The switch is kept in the storage, which gains
  `load_setting` / `save_setting`.
- `delete_secret_chat(chat_id)` and `delete_secret_chat_both_sides(chat_id)`: delete a chat on this
  side only, or also ask Telegram to erase the peer's history (`messages.discardEncryption`
  `delete_history`, as TDLib does). A peer's delete for both sides removes the chat here, and
  `ChatClosedEvent` gains `history_deleted`.
- A stateful property test generates long start/stop/restart, send, TTL, close and
  storage-failure sequences and checks after every step that memory and storage agree,
  closed chats hold no key material, and a returned `start()` leaves a running manager.

### Fixed

- Transient operations have one cancellation deadline owner, preventing a competing timer
  from cancelling a resistant operation twice and misreporting incomplete cleanup.
- Track returned handler Tasks with their wrappers during shutdown: avoid cleanup deadlocks,
  cancellation of a stop caller through its wrapper, and restart from discarded-task cleanup.
- Prevent Python 3.14 shield diagnostics from exposing errors after a cancelled shutdown wait.
- Let cancelled handlers finish without recursively waiting on their own shutdown coordinator;
  reject a restart from that cleanup, and recover a failed shutdown before restarting.
- Retain and retrieve failures from handler Tasks cancelled before their wrapper starts or
  returned while stopping; exception messages never reach the event-loop logger.
- Verify the final ciphertext stream length before replacing a saved file.
- Protect active file/store temporary writers; retain legacy temporary names until offline cleanup.
- Reject stale operations across stop/start, isolate the new send queue, and share concurrent shutdown.
- Preserve a peer discard arriving before the create RPC response; validate persisted runtime fields.
- Validate file-key fingerprints on forwarding and reject malformed serialized media references.
- Enforce declared TL result families in generated readers; persist TTL/layer service effects atomically.

### Changed

- CI explicitly exercises the native `cryptg` backend in addition to the default backend.
- `ManagerStopping` also identifies an operation invalidated by a stop/start lifecycle change.

## [0.2.0] - 2026-09-29

Tagged `v0.2.0`.

### Added

- A pair test tier (`tests/live_pair`): this package on both ends over the real Telegram
  server, two of the owner's accounts, run by `scripts/run_interop.ps1 -PeerAccount`.
  Real-server evidence, not interop evidence.
- The first official-client capture (`tests/vectors/fixtures`), replayed offline.
- `send_file` media metadata: `duration`, `width`/`height`, `thumbnail` + `thumbnail_size`,
  `waveform`, `title`/`performer`, `sticker_alt`, each checked against the kind before upload.
- `send_file` accepts bytes or a seekable stream (with `file_name`) as well as a path.
- `MediaReference` (`MessageReceived.media_reference`): save or forward a received file after a
  restart; `save_file` accepts it.
- `forward_file`: re-send a received file into any chat by its server handle, no upload (ADR 0006).
- `key_visualization`, `KeyVisualization`, `PALETTE`: the key picture as data.

### Changed

- `send_file`'s second parameter is now `source` (was `path`); positional calls are unaffected.
- A failed `discardEncryption` is logged as a warning with its error type.
- A secret chat this session asked for but lost the answer to (unknown waiting or ready chat
  outside `create()`) is discarded on the server and reported as `DecryptFailed`.
- Documented: a peer's close reaches this side on its next update sync or its next send.

## [0.1.0] - 2026-09-29

Tagged `v0.1.0`.

### Changed

- The distribution is renamed to `kiaro-telethon-secret-chat`: `telethon-secret-chat` on PyPI
  is an unrelated package. The import name `telethon_secret_chat` is unchanged; a consumer
  changes its dependency line once.
- `__version__` is read from the installed metadata, so the version is written in one place.
- `list()` and `status()` return a read-only `ChatSnapshot` with no key material instead of
  the live chat object.
- A send whose message was stored but whose transmission failed raises `SendPending` carrying
  the `random_id`; the message is retried by the next send or `start()` and must not be
  resent by the caller.
- Formatted text is parsed with the client's default parse mode, and the entities are mapped
  to the secret-chat schema; entity types the secret-chat layer cannot carry are dropped.
- The in-memory history keeps the most recent `history_limit` messages per chat (default
  1000, a new keyword of `SecretChatManager`).
- Telethon is capped below 2; the build backend is capped below setuptools 86.
- `create()` and `accept()` return a `ChatSnapshot`.
- A message that authenticates but will not parse ends the chat, as TDLib does; a stored
  mailbox item that no longer parses is dropped with `DecryptFailed` instead of blocking
  the messages behind it.
- Files are encrypted and decrypted in pieces (chained IGE) instead of in memory, and a
  download is written to a temporary file beside the target.
- A file upload no longer holds the chat lock: the chat keeps receiving meanwhile, and
  messages sent during the upload still land after the file.
- `retry_pending` sends the records behind a failing one instead of stopping at it.

### Added

- `forget(chat_id)` drops a closed chat's record.
- `UnknownChat` (also a `KeyError`) and `ManagerStopping` (also a `RuntimeError`), so every
  failure is inside `SecretChatError`.
- The `SendFailed(chat_id, random_id, cause)` event for a message Telegram permanently
  rejected; the message is withdrawn as a self-delete that keeps its sequence slot. A send
  answered with a chat-ending error (`ENCRYPTION_DECLINED` and similar) closes the chat.
- The `fast` extra (`cryptg`) for native AES.
- `py.typed`; CI runs a lenient mypy check.
- Crash-leftover temporary files beside the store (`.secret-chat-store-*.tmp`) and beside a
  saved file (`.secret-chat-file-*.tmp`) are deleted on the next open or save.

### Fixed (since the tree first pinned by the consumer, 42fd06f)

- Rekey and resend recovery hardened; remaining audit gaps closed (2026-09-26).
- `start()` validates every stored record and raises `StoreCorrupt` before installing any
  chat; `create()` discards the server-side request when its record cannot be saved; service
  actions rekey first when the trigger is due (2026-09-21 to 2026-09-26).
- Durability and protocol repairs from the audit, with strict TL parsing regenerated from the
  tracked schema (2026-09-21).

## [0.0.1] - 2026-09-19

- First scaffold; the tree the consumer first pinned is `42fd06f` (2026-09-21).
