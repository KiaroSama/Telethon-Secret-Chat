# Durable ingress and strong-ingress mode

`StrongSecretChatManager` in [runtime_modes.rs](../src/runtime_modes.rs) couples `PersistentSecretChatManager` with `DurableIngressController` and `EncryptedIngressStore` in [ingress.rs](../src/ingress.rs).

## Core API and invariants

- **Pre-acknowledgement capture**: `ingest_before_ack` validates provenance (`IngressOrigin::VerifiedTransport` / `LiveSocket` / recovery origins), commits the encrypted durable ingress record atomically before transport acknowledgement, and deduplicates exact identities without consuming extra slots.
- **Strict deserialization and stale-revision protection**: Snapshot loading rejects duplicate keys and numeric aliases (PR #51 R06); empty or non-empty drains and acknowledgements verify the current on-disk ciphertext revision first (PR #51 R07).
- **Contiguous applied cursor**: `applied_cursor(stream_id)` tracks the highest contiguous 1-based settled order; `acknowledge_contiguous(stream_id, order)` refuses out-of-order acknowledgement ahead of an unapplied gap.
- **Bounded polling**: `list_pending_page(stream_id, offset, limit)` returns detached pending records (`limit` 1..=100, `offset` 0..=4096) without acknowledging or mutating state.
- **Isolated startup replay**: During restart replay, stale pending records targeting a closed or non-ready chat are isolated per chat so they never block startup of other healthy chats.

## Persistence boundaries

Encrypted ingress snapshots use explicit absolute paths, exclusive staging, atomic replacement, and empty-lockfile verification (PR #51 R05). Callers own directory ACLs, encryption key lifecycle, and any anti-rollback anchor.
