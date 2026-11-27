# Durable ingress records and the strong-ingress boundary

The current [Rust ingress module](../src/ingress.rs) provides `EncryptedIngressStore` and `DurableIngressController`. They protect captured opaque records across reconstruction. They do **not** implement the retired Python `StrongSecretChatManager`, a verified transport bootstrap, or a complete transport-to-protocol durable transaction.

This distinction matters because Telegram's secret-chat queue is authorization-wide. A qts supplied to `updates.getDifference` can acknowledge earlier events, as can `messages.receivedQueue`; local post-delivery persistence cannot undo a prior server acknowledgement. See [Telegram's update-box contract](https://core.telegram.org/api/end-to-end#working-with-an-update-box).

## Actual Rust API

| API | Effect |
| --- | --- |
| `EncryptedIngressStore::open(path, key)` | Select an absolute file path and caller-supplied encryption key. The parent directory must exist. |
| `DurableIngressController::new(owner, store, limits)` | Start fresh state. Its first write refuses an existing snapshot; use `restore` for existing custody. |
| `DurableIngressController::restore(owner, store, limits)` | Authenticate and validate a snapshot, its owner, limits, record identities, and deduplication membership. Pending records are marked replayed. |
| `capture_and_commit(update)` | Validate and durably capture a new identity before adopting it in RAM. Exact duplicates do not consume another pending slot. |
| `capture_with_barrier(update, barrier)` | Await a caller barrier before synchronous capture. Cancellation during the barrier does not commit the new record. This is not a transport attachment. |
| `pending_in_order(stream)` | Return detached, sorted pending records without removing them. |
| `acknowledge(stream, order)` | Persist removal of one consumer-settled record before updating RAM. Already settled known identities are idempotent; unknown identities refuse. |
| `drain_in_order(stream)` | Atomically acknowledge and return the whole currently pending stream. Returns `Result`; failures preserve pending custody. |

For recoverable consumer processing, poll with `pending_in_order`, durably commit the consumer's own result, then acknowledge the stable `(stream_id, order)` identity. External side effects need their own stable idempotency identity. The library does not atomically commit an external database or Telegram cloud send.

`drain_in_order` is a destructive bulk acknowledgement, so a crash after it returns but before the caller processes the returned values can lose that consumer work. Its name describes sorting within that snapshot, not an at-least-once consumer guarantee.

An `IngressOrigin` value records the caller's declared provenance. The enum does not prove that bytes came from a verified live socket or complete difference response.

## Ordering and cursor limitations

The controller accepts out-of-order identifiers and sorts the currently pending set. It has no verified initial sequence, contiguous applied cursor, or missing-order barrier. For example, a snapshot containing orders 1 and 3 can be drained before order 2 is captured. Sorting each snapshot does not establish global replay continuity.

The `seen_orders` history supports exact identity deduplication, including already acknowledged identities. It is not a trusted Telegram session checkpoint or proof that every preceding update was received.

The controller does not:

- install the vendored transport's capture/acknowledgement hooks;
- establish a verified qts baseline or reconstruct a complete difference cursor;
- commit secret-chat counters, rekey state, outbox, gaps, and an applied cursor together;
- project protocol messages into durable application relay obligations;
- attach to a caller's manager lifecycle or resume paused protocol participation.

The transport patches in [vendor/grammers](../vendor/grammers/README.md) are prerequisites/components, not evidence that those integrations exist.

## Budgets

| Budget | Default or ceiling |
| --- | ---: |
| Configured `max_pending_records` | Default 1024; positive, at most 4096 |
| Configured `max_payload_bytes` | Default and ceiling 1 MiB |
| Aggregate pending payload | 16 MiB |
| Streams | 4096 |
| Retained deduplication identities | 65,536 |
| Owner and stream identity text | At most 1024 bytes per identity |
| Encrypted serialized document | 64 MiB |

Capacity exhaustion refuses new identities without evicting prior obligations or deduplication history. A duplicate can still succeed when the pending queue is full. Conflicting content for an identity that is still pending refuses. Reaching the deduplication ceiling is an explicit operational stop; the library does not silently reset a replay baseline or rotate away identity history.

These are container/storage bounds, not a process-RSS, transport-queue, operation-time, record-age, or consumer-task guarantee.

## Persistence and failures

AES-256-GCM protects the snapshot, including payloads. A sidecar file lock coordinates mutation and a ciphertext revision check prevents a fresh or stale controller from overwriting another committed snapshot. Use one logical owner and reconstruct deliberately after a stale-writer refusal.

Writes use exclusive random staging, file flushing, replacement without an unlink gap, and Unix parent-directory flushing. A failed acknowledgement leaves the in-memory obligation available. Uncertain post-replacement durability makes the handle refuse further work until reopening and reconciliation.

The caller owns encryption-key lifecycle, private directory permissions, Windows ACLs, backups, and anti-rollback policy. AEAD detects unauthenticated modification; it does not detect replacement with an older authentic file without an external trusted anchor. No physical-erasure or universal hardware power-loss guarantee follows.

Telegram may remove acknowledged events or events older than seven days, and deauthorization clears the authorization's queue. These local containers cannot extend those server retention limits or recover work never captured. [Primary server contract](https://core.telegram.org/api/end-to-end#working-with-an-update-box).

## Migration and evidence

`drain_in_order` now returns `Result<Vec<DurableIngressRecord>, SecretChatError>`. Callers must propagate or handle the durable acknowledgement error. The former implementation removed RAM state and suppressed save errors; code depending on an unconditional `Vec` must change.

Relative file paths, invalid restored limits/state, mixed identities, and stale snapshot writes now refuse. Existing Python `prepare`, patched-prerequisite validation, baseline, and strong-manager examples belong to the historical Python implementation and are not callable Rust APIs.

Current tests cover capture/reconstruction, corruption and owner refusal, duplicate/backpressure behavior, cancellation before capture, stale writers, failure-preserving acknowledgement, and non-consuming polling. They do not establish live transport attachment, protocol-applied cursor integrity, complete manager restart, or exactly-once external delivery.
