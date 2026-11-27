# Durable ingress and strong-ingress mode

`StrongSecretChatManager` in [runtime_modes/mod.rs](../src/runtime_modes/mod.rs)
couples `PersistentSecretChatManager` with `DurableIngressController` and
`EncryptedIngressStore` in [ingress/mod.rs](../src/ingress/mod.rs).

## Capture, processing, and consumer settlement

1. `ingest_before_ack` accepts only `IngressOrigin::VerifiedTransport`, a positive
   chat identity, its matching `secret-chat/{id}` stream, and positive order. The
   transport adapter must establish provenance before using that marker. Generic
   controller support for other origins does not authorize them in Strong mode.
2. Capture durably commits the original ciphertext before the transport may
   acknowledge it. The caller maps Telegram's device-wide update sequence to
   these per-chat, contiguous, 1-based delivery orders. This cursor is not `qts`.
3. `apply_delivery` and `replay_pending` process the next contiguous delivery.
   Protocol state, response outbox, and its accepted message batch commit
   together. Repeated application or restart returns the same retained batch
   until the consumer acknowledges it, including all messages released by a gap.
4. The consumer performs its application work, then calls
   `acknowledge_delivery`. The ingress cursor commits first; receipt cleanup
   follows. If cleanup fails, retry acknowledgement or the next apply/replay
   operation. A completed cursor is not rolled back and already settled messages
   are not redelivered. Consumers may also process the raw polled ciphertext and
   acknowledge it directly; ACK is the consumer's assertion of settlement.

An application crash after its side effect but before ACK can cause redelivery.
Use idempotent effects or an application transaction keyed by delivery identity.
Plain `PersistentSecretChatManager::handle_incoming` instead settles its receipt
before a successful return; retry the exact original ciphertext after an
interrupted call. Neither API creates an exactly-once transaction with external
application effects.

## Bounds and failures

Accepted batches use `ManagerLimits` count and byte budgets. Refusal occurs before
consuming protocol state; existing obligations remain intact. Response RPC
failure preserves both the batch and retryable outbound ciphertext. A successful
receive therefore does not imply every response RPC succeeded; use
`retry_pending` when needed.

`list_pending_page(stream_id, offset, limit)` is non-consuming (`limit` 1..=100,
`offset` 0..=4096). Snapshot loading refuses duplicate identities and inconsistent
state. Operations verify stored revisions before replacing snapshots. Startup
replay isolates closed, unknown, and not-ready chats and reports per-chat receive
errors; a storage failure may still fail the operation itself.

Encrypted files require explicit absolute paths, private parent directories,
exclusive staging, cooperative lockfiles, and caller-owned key custody. If a
mandatory closure occurs while storage is unavailable, RAM remains closed, but
the previous disk checkpoint can only be replaced after storage recovers. Retry
`close_and_tombstone` before treating closure as restart-durable. See
[integration boundaries](integration-boundaries.md) for authorization upgrades.
