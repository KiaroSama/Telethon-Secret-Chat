# 0001. An explicit storage backend, atomic across the record and both queues

- Status: accepted
- Date: 2026-09-28 (decided 2026-09-19 in the feature spec)

## Context

A secret chat's key, its counters, the messages kept for a resend (§3.7) and the messages
waiting behind a gap must all survive a restart; a counter saved without its message is as
corrupt as a key saved without its fingerprint. The archived base package saved on every
attribute write, and a library that picks a storage location silently picks one the operator
never protected.

## Decision

`SecretChatManager(client, storage)` takes a `StorageBackend` and refuses to start without one
(`StorageRequired`). A chat is saved as one whole record, and `transaction()` makes the record
and both queues change together: the manager's `_atomic` step runs one transaction and rolls
the in-memory chat back with it. Transactions are synchronous and never span a network await.
`MemoryStorage` and `FileStorage` ship; anything else implements the contract in
[architecture.md](../architecture.md#3-the-storage-contract).

## Consequences

- No accidental key file in the working directory; every application decides where keys live.
- A custom backend must provide real transactions and store `bytes` and 2048-bit integers
  losslessly; compensating writes after a failure do not meet the contract.
- `FileStorage` rewrites the whole store per commit: simple and crash-safe, with a cost that
  grows with the store (measured in `docs/design/storage-scaling.md`).
