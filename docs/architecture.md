# Current Rust architecture and contracts

This page describes the compiled Rust implementation. The retired Python package had broader managers, storage backends, media operations, and archive APIs. Historical ADRs and the [migration receipt](../migration/019-rust-anything-receipt.json) retain that context; they do not supply missing Rust behavior.

## Module boundaries

| Module | Responsibility and boundary |
| --- | --- |
| [protocol/mod.rs](../src/protocol/mod.rs) | Live chat keys, layers, sequence state, authenticated frames, gap queues, and batch acceptance. No manager restart serialization. |
| [ported/rekey.rs](../src/ported/rekey.rs) | Runtime DH configuration and RequestKey/AcceptKey/CommitKey/Noop exchange, collision/abort handling, key selection, and retirement. Also contains historical adapter helpers. |
| [manager/mod.rs](../src/manager/mod.rs) | In-memory chat registry, retained outbox, per-chat operation coordination, transport calls, and queued events. |
| [custody.rs](../src/custody.rs) | Payload-count/byte limits and encrypted snapshots of custody records. Not a protocol storage backend. |
| [ingress.rs](../src/ingress.rs) | Opaque ingress record capture, duplicate identities, sorted polling, durable acknowledgement, and reconstruction of that container. |
| [storage_file.rs](../src/storage_file.rs) | Shared private file locking, bounded reads, exclusive temporary creation, flushing, and atomic replacement. |
| [export.rs](../src/export.rs) | Reusable formatting, URL/MIME, ICC, and JPEG-related helpers. No full saved-chat archive reader/writer. |
| [lifecycle.rs](../src/lifecycle.rs) | Wrappers around selected action, history, lock-check, and temporary-file adapter behavior. These are not complete runtime lifecycle subsystems. |
| [public_api.rs](../src/public_api.rs) | Historical-name metadata with explicit availability status. Not type aliases or Rust re-exports. |

The Rust package has no Python runtime. The JPEG dependency uses native mozjpeg C code, and dependencies may contain unsafe code. The crate-level unsafe prohibition is scoped to this crate.

## Transport and establishment

The caller owns the authenticated client and implements `SecretTransport`. The crate re-exports grammers crates for integration, but does not automatically implement the adapter, manage login, install update handlers, or recover transport state.

The manager starts an existing chat through `register_established_chat`, using caller-supplied identifiers, role, and shared key. Arithmetic and validation helpers support DH, but the initial network request/accept handshake is not a public manager workflow. Applications must not treat key installation as proof that the corresponding server-side handshake or authorization binding was verified.

Protocol service messages require `SecretTransport::send_encrypted_service` and Telegram's `messages.sendEncryptedService` RPC. The trait's default implementation refuses service delivery; a caller adapter must implement it. Ordinary message and discard RPCs remain separate operations.

## Live protocol behavior

The protocol core authenticates the encrypted frame before applying sequence/layer effects. It enforces wrapper and TL boundaries, sender parity, acknowledgement bounds, replay checks, and separate peer capability versus wrapper-layer monotonicity. Authentication failures do not justify trusting decoded fields.

Out-of-order authenticated frames are retained within finite gap bounds. Filling a hole may make several messages ready. Use `receive_encrypted_frames` at the protocol layer or `receive_frames` at the manager layer to observe the whole batch. A compatibility single-message receive method cannot communicate an arbitrary batch through one return value.

The runtime retains resend state and distinguishes protocol actions that must be handled immediately from ordinary gap-delayed delivery. Unsatisfiable retransmission and authenticated fatal protocol violations fail closed. Receiving ciphertext is not equivalent to acknowledging application delivery.

Runtime rekey support includes configured DH validation, signed exchange-ID collision resolution, RequestKey/AcceptKey/CommitKey/Noop actions, and active/pending/previous key handling. A correctly authenticated pending-key message can confirm a switch when a Commit message is delayed. Sequence counters survive a key change. Previous keys retire only under the implemented confirmation and gap conditions. The protocol exposes a rekey trigger based on message use and age; the manager coordinates service frames with its retained outbox. Initial network establishment and durable reconstruction of an in-progress exchange remain separate, unsupported workflows.

Call `configure_chat_dh` with the original validated parameters before the automatic threshold. `start_rekey` initiates an exchange explicitly; `send_service_action` accepts supported conversation actions while the state machine owns rekey and recovery actions. Missing required DH parameters fail closed rather than allowing indefinite use of a key. Automatic age evaluation happens during operations, not in a background timer.

The protocol rules and supporting sources are in [protocol-reference.md](protocol-reference.md). That reference is broader than the implemented product surface; it is not a feature checklist.

## Locks, sends, and cancellation

Each chat has a state mutex and a separate outbound-order mutex. Outbound serialization prevents concurrent sends from reserving the same sequence or overtaking earlier pending traffic. State changes do not hold the chat state mutex over network I/O, so receive processing and unrelated chats can progress.

Before a send awaits transport, its original ciphertext, random ID, and sequence reservation are retained in RAM. An uncertain transport result or cancellation does not roll back the possibly delivered sequence. Retrying uses the original bytes and identity. Successful RPCs remain retained until authenticated peer acknowledgement releases them.

These are memory transitions, not disk transactions. The manager does not call `CustodyStore` or `DurableIngressController` to make its complete state durable. Losing the process loses its keys, live exchange state, outbox, gaps, and events unless the caller supplies a separately designed and verified integration.

Manager close and shutdown are terminal for that manager instance. They do not undo an RPC that has already crossed the transport boundary. Caller-owned futures and external side effects require caller coordination; the manager does not claim a general hard cancellation deadline.

## Events and application settlement

Events are queued in memory and removed by `drain_events`. Acceptance and event insertion avoid an intervening cancellation point, but event draining is not transactional with an application's database or remote send.

`SecretEvent` includes chat-ready/closed, ordinary message, service-action, and peer-acknowledgement events. An inbound transition and its events can commit in RAM before an automatic service reply encounters a transport failure. `receive_frames` then returns `SendPending` while the reply remains retained. `drain_events` is the authoritative notification queue for that accepted inbound result; retry the original reply with `retry_pending` rather than assuming the input was never applied.

The registered-chat map and event queue do not have a global capacity budget. The caller must bound registrations and regularly drain/process events. Registered IDs, including closed-chat tombstones, remain until the manager is dropped; there is no public per-entry forgetting API. Bound manager-instance ownership accordingly. Per-chat outbox and protocol gap limits do not bound the total process, third-party transport queues, consumer callbacks, or operating-system memory.

Peer sequence acknowledgement, server send acceptance, the application's durable result, a durable-ingress acknowledgement, and a user read receipt have distinct meanings. The caller must establish its own idempotency and settlement mapping for external relays. No read receipt or cloud-send policy follows from draining an event.

## Encrypted stores

Both stores require an absolute filename whose parent directory exists. They serialize an authenticated AES-256-GCM envelope, retain the caller-supplied key in a zeroizing wrapper, and reject invalid or inconsistent restored records. Debug output omits payloads and caller-supplied identifying text.

The file update sequence is:

1. Acquire the persistent sibling `.lock` file's operating-system lock.
2. Validate the current store or, for ingress, verify its expected snapshot revision.
3. Create a unique sibling staging file with exclusive creation and restrictive Unix permissions.
4. Write and flush the complete encrypted envelope.
5. Rename over the destination without first unlinking it.
6. Flush the parent directory on Unix, then adopt the new in-memory state.

A failure before replacement preserves the prior snapshot. A failure after replacement but before confirmed directory durability leaves an uncertain outcome; the store refuses further operations until reopening and reconciliation. Standard file flushing does not imply a guarantee against every filesystem, storage-controller, or power-loss failure.

Locks coordinate cooperating library operations. They are not an authorization system for a directory writable by an adversary. Keep the parent directory private and configure Windows ACLs. The lock sidecar is intentionally not removed while other handles may use it. Temporary cleanup removes only files the current operation created; crash leftovers require deliberate offline maintenance.

Custody cleanup authenticates the store before deleting it, and preserves unrelated or unauthenticated files. Neither cleanup nor zeroization promises physical erasure.

See [bounded custody](bounded-mode.md) and [ingress storage](strong-ingress.md) for their exact data models, ceilings, acknowledgement semantics, and API migration notes.

## What restoration does not mean

A reconstructed custody record contains payloads, limits, authorization text, and a pause flag. It does not contain a complete `SecretChat`.

A reconstructed ingress controller contains captured opaque records and deduplication identities. It does not verify a Telegram baseline, install qts acknowledgement barriers, apply secret-protocol state transactionally, or maintain a contiguous applied cursor. Sorting currently pending records cannot establish that no earlier update is missing.

Consequently, neither API is a replacement for the retired `StorageBackend`, `FileStorage`, `ProtectedFileStorage`, `BoundedSecretChatManager`, `TransientSecretChatManager`, or `StrongSecretChatManager`. Those names in a historical inventory are not importable Rust compatibility types.

## Verification and historical evidence

Use `cargo test --locked --all-targets`, strict Clippy, formatting checks, and the CI result for the exact commit. Tests exercise native code, synthetic transport failures/cancellation, storage fault boundaries, protocol vectors, and export goldens. Fixture coverage is not a live Telegram or consumer deployment result.

The migration receipt is preserved verbatim as a historical record of reported adapter gates and symbol reconciliation. Its totals do not establish complete native feature coverage, and a test that checks nonempty path strings cannot prove those paths resolve or preserve behavior. The current public inventory identifies those gaps explicitly, while compile-checked tests exercise concrete Rust items.
