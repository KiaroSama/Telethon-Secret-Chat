# Changelog

Every change to the public API gets a line here in the same commit. The consumer pins this
package by commit and reads this file before moving its pin. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) with the pre-1.0 rule that a minor bump may break.

## [Unreleased]

### Added

- A pair test tier (`tests/live_pair`): this package on both ends over the real Telegram
  server, two of the owner's accounts, run by `scripts/run_interop.ps1 -PeerAccount`.
  Real-server evidence, not interop evidence.
- The first official-client capture (`tests/vectors/fixtures`), replayed offline.

### Changed

- A failed `discardEncryption` is logged as a warning with its error type.
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
