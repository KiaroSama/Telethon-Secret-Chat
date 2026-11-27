# Encrypted checkpoint storage

## What the benchmark measures

Run the integration test through the normal Rust CI suite. On an approved test
runner, show the measurements with:

```sh
cargo test --locked --test storage_checkpoint_bench -- --nocapture
```

[`storage_checkpoint_bench.rs`](../../tests/storage_checkpoint_bench.rs) runs
four independent stores with 1, 16, 64, and 128 sequential incoming messages.
Each run starts with the same synthetic chat and processes 1 KiB of text per
message. It uses no network, real account, or live chat. The message IDs, text,
and sequence are fixed; MTProto padding, store nonces, and checkpoint tokens
still use production randomness. Ciphertext and some JSON byte counts can differ
between runs.

`PersistentSecretChatManager` first saves the accepted batch, chat counters, and
keys together. That commit leaves a receipt for recovery if the call is
interrupted. A second checkpoint removes the receipt before a successful
return. The timing harness wraps `FileStorage::save_checkpoint`, measuring its
read, validation, serialization, encryption, and replacement time. Outside
that timer it opens every checkpoint anew, verifies the complete saved record,
and records the serialized snapshot and receipt sizes. The encrypted file has
36 bytes of overhead: 8 bytes of magic, a 12-byte nonce, and a 16-byte GCM tag.

The output includes total wall time, average wall time per message, total
checkpoint time and its average per message, and p95 checkpoint latency. Byte
counts include total, average, and peak snapshot JSON bytes plus total receipt
JSON bytes. Wall time for each scale
includes the verification reads. The 120-second test-wide ceiling guards
against major regressions; it is not a storage latency target. This workload
exercises receipt creation and removal, not accumulation of a large backlog.

## When to keep snapshots

`FileStorage` authenticates the snapshot under AES-256-GCM with the store magic
and owner binding as associated data. A writer verifies the current checkpoint
under a persistent, empty sidecar lock and stages the entire replacement in
the same directory. It syncs the staged file before atomic rename and syncs the
parent directory on Unix. A pre-rename failure leaves the prior snapshot in
place. A reported post-rename sync failure leaves the outcome uncertain; reopen
and verify the authenticated state before retrying. Keep the directory private
and never delete or repurpose the lock file while a store may be in use.

For a small store with promptly settled messages, a snapshot keeps the durable
state together in one file with one replacement protocol. Its per-checkpoint
work scales as `O(active_chats + pending_outbox + pending_receipts)` for fixed
record sizes, or with total serialized bytes when records vary. Active state
also includes tombstones, suspension markers and buffered protocol data. Each
checkpoint reads, decrypts, and validates the old snapshot and serializes,
encrypts, writes, and syncs the new one. The manager separately serializes the
pending receipt array to enforce its byte budget. Retained receipt text and the
raw message bytes appear together; JSON encodes the byte arrays as lists of
numbers. With thousands of large retained receipts across chats, this duplicates
serialization and allocation work and rewrites unchanged data under the same
writer lock. As the backlog grows, accumulating `N` equally sized receipts can
cause `O(N^2)` cumulative bytes rewritten. Sync latency and contention can
become limiting before the 64 MiB file cap.

## Upgrade trigger and design

Profile on the deployment's own durable volume when snapshots reach 8 MiB or
an owner regularly holds 1,000 pending receipts. Prototype an incremental
backend if p95 `save_checkpoint` exceeds 50 ms in three representative runs
with a settled load or realistic backlog, or if projected snapshots exceed
32 MiB (half the current 64 MiB cap). These are proposed investigation and
headroom thresholds, not benchmark results or service promises. Record the
filesystem, hardware, receipt size distribution, and concurrency alongside
measurements. Reducing retention is only safe after consumers settle their
obligations.

Use one transaction per state transition. Store chat rows, outbox rows, receipts,
tombstones, and suspension markers independently; key their identities by the
verified owner. A SQLite implementation can use WAL on a local filesystem with
`PRAGMA synchronous=FULL`. Keep keys and payloads in application-encrypted blobs
with authenticated owner, format, row kind, row ID, and revision; a standard
SQLite WAL has no built-in payload encryption. Protect the database and WAL
sidecars under the same private directory. Inside a write transaction, verify
the current checkpoint (or absence on insert), then update the chat, increment
its revision/rotate its checkpoint token, and apply the matching outbox and
receipt changes. A stale comparison must roll back the whole operation. Retain
a separate, persistent empty lock file for migration and maintenance; neither
the database nor its WAL can be used as that lock.

Without a database, append bounded authenticated frames with the same owner and
expected/new revision, plus length, operation kind, and a fresh nonce. Write
one transaction frame for the full state transition and sync it before
reporting success. Replay contiguous committed revisions only. An incomplete
trailing frame is recoverable; authentication failure in a complete frame or
a missing intermediate revision must stop recovery. Compact to a synced,
atomically installed snapshot under the same empty lock before retiring old
journal data.

For either design, import a validated snapshot under the maintenance lock and
verify owner, counters, keys, outbox, receipts, and replay state before switching
writers. Do not enable both backends for writes. Test competing checkpoint
writers, a nonempty or aliased lock sidecar, and interruption before and after
commit, recovery, and compaction. Do not expose accepted messages or allow
transport ACK before the matching durable write. Keep consumer ACK and receipt
cleanup retryable in the current order. Retain a ciphertext backup through
cutover verification; after new writes, reverting to that backup alone loses
committed state.
