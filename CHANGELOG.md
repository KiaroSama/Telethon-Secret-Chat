# Changelog

Every change to the public API gets a line here in the same commit. The consumer pins this
package by commit and reads this file before moving its pin. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/) with the pre-1.0 rule that a minor bump may break.

## [Unreleased]

### Added

- Deterministic property-based state-machine tests (`tests/property_state_machine.rs`)
  covering message reordering, replays, gap filling, PFS rekey cutovers, restart
  checkpoints, and Strong-mode contiguous acknowledgement invariants.
- Versioned typed protocol snapshots (`protocol::typed_state::{TypedPersistedProtocol,
  PersistedGapState, PersistedRekeyExchange, migrate_json_v1_to_v2, decode_v2_json}`)
  making illegal gap and rekey combinations unrepresentable while preserving `v1`
  snapshot round-trip compatibility (`tests/typed_persisted_state.rs`).
- Reproducible whole-file checkpoint benchmark suite (`tests/storage_checkpoint_bench.rs`)
  and sizing guide (`docs/benchmarks/checkpoint-storage.md`) documenting when whole-file
  atomic snapshots suffice versus when append-only or WAL storage is warranted.
- Explicit offline `v1`-to-`v2` binding migration helpers (`binding_migration::{
  BindingOwnershipProof, MigrationSummary, migrate_chat_store_v1_to_v2,
  migrate_custody_store_v1_to_v2, migrate_ingress_store_v1_to_v2}`) with strict
  caller-supplied old/new identity verification (`tests/binding_migration.rs`).
- Caller-owned orchestration helpers (`orchestration::{MediaBudgets, MediaOrchestrator,
  SentEncryptedMedia, AutoSaveSubscription, BoundedCustodySettlement, RichTextEntity,
  ServiceArchiveEvent, RichArchiveRecord, render_rich_html, export_rich_saved_chat}`)
  for bounded encrypted media transfer, per-chat autosave routing, rich entity/service-event
  archive export, and in-memory bounded delivery settlement (`tests/orchestration_parity.rs`).
- Opt-in manual live interoperability workflow (`.github/workflows/live-interop.yml`)
  and three-tier verification guide (`docs/interop-testing.md`) separating offline
  vectors, same-library live pairs, and official-client interoperability.

### Fixed

- Resolve implicit text TTL at sequence reservation after queued transmissions,
  and keep caller-ID retries stable across TTL changes, restart, and key retirement.
  Preserve explicit RAM TTL and legacy digests; see the
  [follow-up audit](docs/audits/2026-11-28-followup.md) for legacy recovery limits.
- Disable inaccessible duplicate events at persistent/transient manager construction,
  while preserving the direct RAM manager's public event stream.
- Keep thumbnail ICC/scaling pixels in packed buffers. A valid 24 MP JPEG now has
  regression coverage through both thumbnail and archive paths in a 512 MiB child.
- Bind `GrammersTransport` to the actual local authorization-key tag, verified user,
  home DC, and generation; accept current-session management hash `0`, and guard
  RPC, multipart, download, and handshake await boundaries against identity changes.
- Refuse oversized `EstablishedChat::encrypt_text` output before consuming a sequence.
- Checkpoint durable manager transitions before RAM publication; retain accepted
  receive batches across save refusal, response failure/cancellation, and restart.
  Strong mode retains results through explicit consumer ACK, with bounded receipts
  and retryable cleanup. Mandatory closure remains terminal in RAM during save failure.
- Propagate memory-store transient marker errors, use authoritative transient TTL,
  and avoid inaccessible duplicate RAM events in durable managers.
- Reject inconsistent restored counters, pending wrappers, replay/key provenance,
  recovery windows, and gap metadata before saving or installing protocol state.
- Refuse duplicate identities and unknown fields in authenticated autosave history
  without changing public message/media Serde behavior or valid writer output.
- Stage complete archives before publication, clean failed output, validate media
  forwarding locations/sizes, and require complete Ogg Opus identification headers.

### Compatibility and documentation

- Authorization binding domain `v2` intentionally differs from the old adapter's
  management-hash identity. Existing adapter-bound stores are not automatically
  rebound. See [integration boundaries](docs/integration-boundaries.md).
- Existing valid checkpoint records remain readable under an unchanged binding;
  absent accepted-batch journals default to empty. Previously accepted malformed
  snapshots now fail closed. Public RAM-manager signatures and event behavior remain.
- Document caller-owned initial handshake futures, explicit autosave recording,
  helper-level media composition, archive export scope, and consumer cursor semantics.
  Correct module links and CI/release claims. The immutable `v0.4.0` tag is unchanged.

## [0.4.0] - 2026-11-27

### Added

- Full initial secret-chat network establishment and caller-owned Layer 234 `grammers_client::Client` transport adapter (`transport::GrammersTransport`, `transport::EstablishmentTransport`, `establishment::SecretChatEstablisher`: `request_chat`, `complete_outgoing_handshake`, `accept_chat`, `discard_chat`, `delete_chat_both_sides`, plus dependency-free 12x12 key visualization PNG renderer `render_key_visualization_png`).
- Full persistent, restartable secret-chat storage and runtime mode managers (`storage_backend::{SecretChatStore, MemoryStorage, FileStorage, ProtectedFileStorage, PersistedChatRecord}`, `runtime_modes::{PersistentSecretChatManager, BoundedSecretChatManager, TransientSecretChatManager, StrongSecretChatManager}`) preserving keys, sequence counters, TTL, PFS rekey state, previous-key cutover intervals, and retained outboxes across process restarts with atomic compare-and-swap checkpoints.
- Contiguous durable ingress cursor (`applied_cursor`, `acknowledge_contiguous`, `list_pending_page`, `IngressOrigin::VerifiedTransport`) and isolated per-chat closed-chat startup replay handling.
- Full encrypted media support for all 8 secret-chat media kinds (`media::{SecretMediaKind, encrypt_media_payload, decrypt_media_payload, encrypt_media_stream, decrypt_media_stream, build_forward_media_descriptor, detect_voice_note, save_media_atomic}`), opt-in encrypted atomic `autosave::AutoSaveStore`, and complete Telegram Desktop-compatible HTML + JSON saved-chat archive export (`export_archive::export_saved_chat`).

### Fixed

- Integrated PR #51 (`12125bb2`, R01-R10): previous-key sequence interval cutover, buffered neighbor acknowledgement and wrapper-layer monotonicity checks, duplicate authenticated wrapper drop before inner TL decode, owner-bound `CustodyStore`, empty-lockfile integrity check, strict serde duplicate/alias rejection, stale-revision check before empty `drain_in_order`, strict schema generator syntax validation, tracked live-secret filename checks in `validate-source.sh`, and Jiff `1.0.3` upgrade (`parse_zoned_datetime`) in vendored `grammers-client` (superseding PR #50).

### Changed

- Renamed repository and all project references to **Grammers Secret Chat** (`KiaroSama/Grammers-Secret-Chat`).
- Ignored `.cargo/` completely in `.gitignore` (`/.cargo/` and `**/.cargo/`) and split oversized Rust modules below 700 lines.

## [0.3.2] - 2026-11-27

### Fixed

- Preserve encrypted custody across failed updates with exclusive staging files, atomic replacement without deleting the previous destination, file/Unix-directory flushing, cooperative file locks, and stale ingress snapshot refusal.
- Preserve retained custody at capacity, reject volatile records at the disk boundary, validate restored identity/accounting/limits, and authenticate files before cleanup.
- Make ingress acknowledgement failures observable: `drain_in_order` now returns `Result`; add non-consuming polling and explicit durable acknowledgement so consumers can settle work before releasing it.
- Retain original outbound frames and sequence reservations through transport failure or cancellation, serialize sends, preserve authenticated gap batches, and enforce wire/schema/padding boundaries.
- Implement live runtime rekey and service transport handling, with key-transition and outbox capacity checks. This does not add initial network establishment or persisted manager restoration.
- Redact payload-bearing diagnostics and correct the public media vocabulary and key-visualization palette.

### Documentation and API scope

- Replace claims of complete Python-to-Rust parity with an explicit current capability inventory. `PUBLIC_EXPORTS` is descriptive migration metadata, now including availability status and notes; it does not define aliases for missing Rust managers, backends, media types, or events.
- Document current custody/ingress limits, fallible drain migration, service transport requirements, volatile manager state, consumer settlement, and unsupported native media/archive and protected-mode workflows.
- Correct the protocol reference: received MTProto 2.0 padding must be 12 through 1024 bytes; TDLib's larger total encrypted-size buckets do not permit larger padding.
- Preserve the historical migration receipt and Python feature notes as evidence, with explicit qualification that adapter counts and recorded gates are not proof of complete native runtime parity.

## [0.3.1] - 2026-11-27

The migration announcement and Python feature notes below are historical. The current Rust scope and audit corrections are stated above and in [README.md](README.md). In particular, inventory totals do not establish that all baseline APIs or workflows are callable in Rust.

### Changed

- Recorded migration adapter work for the 92-file frozen Python baseline `5b68ef95bacbcc6d520f960899fca58ad447e7ea` through `rust-anything 5.0.1` in `migration/019-rust-anything-receipt.json`. The Rust package removed PyO3/Python runtime dependencies and forbids unsafe code in its own crate. The inventory is not a complete feature-parity proof, and mozjpeg remains a native C dependency.
- Synced vendored `Lonami/grammers` crates under `vendor/grammers/` to upstream commit `11e13a989a754f9cf01a798480a2e20ce9e5b1fd` (Telegram API Layer 234, including TCP `nodelay`) while preserving local credentialed SOCKS5 and durable-ingress patches across 88 verified SHA-256 hashes.
- Added Rust `check-test-matrix` and `generate-schema` maintenance binaries with configured 64-bit Linux/Windows build, signing, and provenance workflows. Actual published assets and successful checks must be verified on their release revision.

### Historical Python feature notes

These notes describe capabilities of the retired Python baseline. They are preserved for historical context and are not current Rust API documentation; several named managers and media/archive APIs below are unavailable in the Rust crate.

- Opt-in strong-ingress contracts: `EncryptedIngressStorage`, `DurableIngress`,
  `IngressLimits`, `IngressBatch` and `StrongSecretChatManager`, plus a checksum-pinned
  client prerequisite builder (replaced in Rust by vendored `grammers`). Public preparation validates a protected baseline
  before connection. The lifecycle/fault matrix and installed-wheel CI passed on
  `f52dd57fa06445875164d45475f29b881a70bdee`; independent source review completed and PR26 merged. The
  optional `strong` extra supplies cryptography 50.0.2. See [status](docs/strong-ingress.md).
- Opt-in `BoundedSecretChatManager` and `CustodyLimits`: isolated pre-effect admission/pause,
  finite durable protocol custody, explicit safe resume, stable consumer relay obligations
  with acknowledgment, and byte-capped RAM media receipt. Refused ingress is not promised
  recoverable; default/transient modes are unchanged. See [contract](docs/bounded-mode.md).
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

- The historical Python distribution was renamed to avoid colliding with an unrelated
  upstream package on PyPI while preserving consumer compatibility.
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
- Legacy Python client dependency was capped below major version 2 and setuptools below 86.
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
