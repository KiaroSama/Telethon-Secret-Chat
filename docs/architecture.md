# Grammers Secret Chat architecture and contracts

This page describes the compiled Rust implementation in `kiaro-secret-chat` (`v0.4.0`). The library uses the caller-owned vendored `Lonami/grammers` Layer 234 client with zero Python or PyO3 runtime dependencies. First-party Rust forbids unsafe code; image processing includes the native mozjpeg dependency. The current feature boundaries are documented in [integration boundaries](integration-boundaries.md).

## Module boundaries

| Module | Responsibility and boundary |
| --- | --- |
| [transport/mod.rs](../src/transport/mod.rs) | Caller-owned `GrammersTransport` wrapping `grammers_client::Client`, async `EstablishmentTransport` trait, Layer 234 encrypted RPCs, encrypted file upload/download, and `Update::Encryption` / `Update::NewEncryptedMessage` adaptation. Reuses the caller client and does not perform login. |
| [establishment.rs](../src/establishment.rs) | Initial secret-chat Diffie-Hellman establishment (`SecretChatEstablisher`: `request_chat`, `complete_outgoing_handshake`, `accept_chat`, `discard_chat`, `delete_chat_both_sides`) and dependency-free 12x12 Telegram key visualization PNG renderer (`render_key_visualization_png`). |
| [protocol/mod.rs](../src/protocol/mod.rs) & [protocol/typed_state.rs](../src/protocol/typed_state.rs) | Live chat keys, layers, sequence state, authenticated frames, gap queues, batch acceptance, PR #51 R01 previous-key cutover, R02 buffered neighbor ack, R03 header dedup before inner TL decode, validated runtime snapshots (`snapshot.rs`), and versioned typed gap/rekey state (`TypedPersistedProtocol`). |
| [ported/rekey.rs](../src/ported/rekey.rs) | Runtime DH configuration and `RequestKey`/`AcceptKey`/`CommitKey`/`Noop` PFS rekey exchange, collision/abort handling, key selection, and retirement. |
| [manager/mod.rs](../src/manager/mod.rs) | Core in-memory chat registry, retained outbox, per-chat serialized outbound locks, transport calls, event queue, and `restore_chat`/`snapshot_chat` hooks. |
| [storage_backend/mod.rs](../src/storage_backend/mod.rs) & [binding_migration.rs](../src/binding_migration.rs) | `SecretChatStore` trait, `MemoryStorage`, `FileStorage`, and `ProtectedFileStorage` backed by atomic AES-256-GCM encrypted persistence (`storage_file.rs`) with R05 empty-lockfile integrity, R06 duplicate-key rejection, account-binding verification, and explicit offline `v1`-to-`v2` store rebinding (`binding_migration`). |
| [runtime_modes/mod.rs](../src/runtime_modes/mod.rs) | Checkpointed `PersistentSecretChatManager` for established chats, `BoundedSecretChatManager`, RAM-only `TransientSecretChatManager` (permanent restart suspension), and `StrongSecretChatManager` coupled to `DurableIngressController` with contiguous applied cursor. |
| [media.rs](../src/media.rs) | All 8 encrypted secret-chat media kinds (`photo`, `video`, `document`, `audio`, `voice_note`, `video_note`, `sticker`, `gif`), AES-256-IGE streaming encrypt/decrypt with pure-Rust MD5 key fingerprint, Ogg Opus voice detection, validated zero-reupload forwarding descriptors, and safe atomic `save_media_atomic`. |
| [autosave.rs](../src/autosave.rs) | Encrypted atomic `AutoSaveStore` (`start_auto_save`, `stop_auto_save`, `record_message`, `read_saved_messages`, `delete_saved_messages`), persisted independently of protocol chat deletion and custody mode. The caller explicitly records messages; managers do not attach autosave automatically. |
| [export.rs](../src/export.rs) & [export_archive.rs](../src/export_archive.rs) | Desktop-style saved plain-text/media exporter (`export_saved_chat`) producing paginated `messages.html`, `result.json`, static assets (`assets/tdexport/`), media copies, and ICC-to-sRGB JPEG thumbnails with strict HTML escaping and URL validation. Staged publication requires a new or empty destination; no append API exists. |
| [orchestration.rs](../src/orchestration.rs) | Caller-owned `MediaOrchestrator` (upload/download/forward with byte budgets), `AutoSaveSubscription`, rich entity/service-event archive export (`export_rich_saved_chat`, `render_rich_html`), and caller-owned in-memory `BoundedCustodySettlement`. |
| [custody.rs](../src/custody.rs) | Bounded payload containers, owner-bound `CustodyStore` (R04), capacity-preserving pause, and explicit resume. |
| [ingress/mod.rs](../src/ingress/mod.rs) | `EncryptedIngressStore` and `DurableIngressController` with R06 strict serde, R07 stale-revision guard, `applied_cursor`, `acknowledge_contiguous`, and bounded `list_pending_page`. |
| [storage_file.rs](../src/storage_file.rs) | Shared private file locking (R05 empty lockfile check), bounded reads, exclusive temporary staging, flushing, and atomic replacement. |

## Security and durability invariants

- Every encrypted frame is authenticated before applying sequence, key-cutover, or layer state transitions. Processed duplicate wrappers drop before inner TL decoding (R03).
- Durable operations checkpoint candidate protocol and outbox state before publishing RAM changes. Outbound ciphertext and sequence reservations are retained before transport and retried with identical bytes and `random_id` on uncertain outcomes. Accepted receive batches remain durable across interrupted calls; Strong mode retains them until explicit consumer ACK.
- All encrypted stores require explicit absolute paths (never defaulting to CWD), atomic replacement with exclusive staging, private Unix file permissions, and cooperative lockfiles. Callers own directory ACLs, encryption key custody, and anti-rollback anchors.
- Zero `unsafe` code (`#![forbid(unsafe_code)]`) across the crate; keys and secrets are zeroized on drop and never formatted into logs or error strings.

See [bounded custody](bounded-mode.md), [transient mode](transient-mode.md), [strong ingress](strong-ingress.md), [checkpoint storage sizing](benchmarks/checkpoint-storage.md), and [interoperability testing tiers](interop-testing.md) for mode, sizing, and verification details.
