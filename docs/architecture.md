# Grammers Secret Chat architecture and contracts

This page describes the compiled Rust implementation in `kiaro-secret-chat` (`v0.4.0`). All major secret-chat subsystems are implemented natively in pure Rust (`#![forbid(unsafe_code)]`) over the vendored `Lonami/grammers` Layer 234 transport with zero Python or PyO3 runtime dependencies.

## Module boundaries

| Module | Responsibility and boundary |
| --- | --- |
| [transport.rs](../src/transport.rs) | Caller-owned `GrammersTransport` wrapping `grammers_client::Client`, async `EstablishmentTransport` trait, Layer 234 encrypted RPCs, encrypted file upload/download, and `Update::Encryption` / `Update::NewEncryptedMessage` adaptation. Never handles login or creates a second Telegram connection. |
| [establishment.rs](../src/establishment.rs) | Initial secret-chat Diffie-Hellman establishment (`SecretChatEstablisher`: `request_chat`, `complete_outgoing_handshake`, `accept_chat`, `discard_chat`, `delete_chat_both_sides`) and dependency-free 12x12 Telegram key visualization PNG renderer (`render_key_visualization_png`). |
| [protocol/mod.rs](../src/protocol/mod.rs) | Live chat keys, layers, sequence state, authenticated frames, gap queues, batch acceptance, PR #51 R01 previous-key cutover, R02 buffered neighbor ack, R03 header dedup before inner TL decode, and `snapshot.rs` lossless runtime state serialization. |
| [ported/rekey.rs](../src/ported/rekey.rs) | Runtime DH configuration and `RequestKey`/`AcceptKey`/`CommitKey`/`Noop` PFS rekey exchange, collision/abort handling, key selection, and retirement. |
| [manager/mod.rs](../src/manager/mod.rs) | Core in-memory chat registry, retained outbox, per-chat serialized outbound locks, transport calls, event queue, and `restore_chat`/`snapshot_chat` hooks. |
| [storage_backend.rs](../src/storage_backend.rs) | `SecretChatStore` trait, `MemoryStorage`, `FileStorage`, and `ProtectedFileStorage` backed by atomic AES-256-GCM encrypted persistence (`storage_file.rs`) with R05 empty-lockfile integrity, R06 duplicate-key rejection, and account-binding verification. |
| [runtime_modes.rs](../src/runtime_modes.rs) | Full restartable `PersistentSecretChatManager`, `BoundedSecretChatManager`, RAM-only `TransientSecretChatManager` (permanent restart suspension), and `StrongSecretChatManager` coupled to `DurableIngressController` with contiguous applied cursor. |
| [media.rs](../src/media.rs) | All 8 encrypted secret-chat media kinds (`photo`, `video`, `document`, `audio`, `voice_note`, `video_note`, `sticker`, `gif`), AES-256-IGE streaming encrypt/decrypt with pure-Rust MD5 key fingerprint, Ogg Opus voice detection, zero-reupload `forward_media`, and safe atomic `save_media_atomic`. |
| [autosave.rs](../src/autosave.rs) | Encrypted atomic `AutoSaveStore` (`start_auto_save`, `stop_auto_save`, `record_message`, `read_saved_messages`, `delete_saved_messages`), persisted independently of protocol chat deletion and custody mode. |
| [export.rs](../src/export.rs) & [export_archive.rs](../src/export_archive.rs) | Complete Telegram Desktop-compatible saved-chat exporter (`export_saved_chat`) producing paginated `messages.html`, `result.json`, static assets (`assets/tdexport/`), media copies, and ICC-to-sRGB JPEG thumbnails with strict HTML escaping and URL validation. |
| [custody.rs](../src/custody.rs) | Bounded payload containers, owner-bound `CustodyStore` (R04), capacity-preserving pause, and explicit resume. |
| [ingress.rs](../src/ingress.rs) | `EncryptedIngressStore` and `DurableIngressController` with R06 strict serde, R07 stale-revision guard, `applied_cursor`, `acknowledge_contiguous`, and bounded `list_pending_page`. |
| [storage_file.rs](../src/storage_file.rs) | Shared private file locking (R05 empty lockfile check), bounded reads, exclusive temporary staging, flushing, and atomic replacement. |

## Security and durability invariants

- Every encrypted frame is authenticated before applying sequence, key-cutover, or layer state transitions. Processed duplicate wrappers drop before inner TL decoding (R03).
- Outbound ciphertext and sequence reservations are retained before awaiting transport and retried with identical bytes and `random_id` on uncertain outcomes.
- All encrypted stores require explicit absolute paths (never defaulting to CWD), atomic replacement with exclusive staging, private Unix file permissions, and cooperative lockfiles. Callers own directory ACLs, encryption key custody, and anti-rollback anchors.
- Zero `unsafe` code (`#![forbid(unsafe_code)]`) across the crate; keys and secrets are zeroized on drop and never formatted into logs or error strings.

See [bounded custody](bounded-mode.md), [transient mode](transient-mode.md), and [strong ingress](strong-ingress.md) for mode-specific guarantees.
