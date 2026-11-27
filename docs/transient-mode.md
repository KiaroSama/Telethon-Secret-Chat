# Transient scope in the current Rust crate

The current [custody module](../src/custody.rs) has a `CustodyMode::Transient` value and memory-only payload containers. It does **not** implement the retired Python `TransientSecretChatManager`, `ProtectedFileStorage`, `TransientLimits`, or `TransientCleanupIncomplete` lifecycle.

## Implemented behavior

A `CustodyRecord` can retain opaque payloads in memory subject to count/byte limits. Capacity refusal preserves existing payloads and pauses the container. `CustodyStore::save_record` refuses records marked transient or memory-only, preventing that API from silently persisting those payloads.

The live `SecretChatManager` also stores its chat keys, retained outgoing ciphertext, gaps, and events in memory. This fact alone does not implement a verified transient policy: the transport and application can copy or persist values, events can remain queued, and there is no protected-state checkpoint or owned-task cleanup coordinator.

The binding-validation and temporary-file functions under `ported` and `lifecycle` exercise selected adapter behavior. They are not a production protected storage backend or a full task/handler lifecycle.

## Unavailable guarantees

| Historical capability | Current Rust boundary |
| --- | --- |
| Protected protocol-only file schema | No native manager store saves only validated keys/counters while excluding every ordinary and service payload. |
| Permanent suspension across reconstruction | No persisted manager reconstruction or suspended-chat registry is implemented. |
| Bounded callback admission and cleanup | No transient handler/task deadline coordinator or incomplete-cleanup result exists. |
| No-payload-write enforcement across media and archives | No native media upload/download/save or auto-save/archive subsystem is supplied. A mode enum cannot enforce policy in caller code. |
| Transient acceptance controls | No initial request/accept network lifecycle is supplied by the manager. |
| Lossless replay after process loss | Volatile records are lost; keys and counters alone cannot reconstruct their original content or retransmission identity. |

Do not recreate an old secret chat with fresh counters after losing its volatile state. Do not interpret installing the same shared key as restoring its sequence, gap, outbox, or rekey history.

## Closing and shutdown

The current manager closes live state and refuses future operations when stopped. It does not implement the historical transient distinction between protected permanent suspension and ordinary remote close. In-flight caller-owned transport work may already have crossed an external boundary; local shutdown cannot retract it.

A memory-only record is not a guarantee against swap, crash dumps, allocator copies, returned values, transport logging, application logging, or application persistence. Zeroizing selected buffers and redacting debug summaries reduce some exposure without proving complete memory erasure.

## Consumer responsibility

The caller owns authorization, consent to initial establishment, read/TTL policy, application storage, and any external forwarding or relay. A cloud copy remains a separate external effect and storage decision.

No native Rust example should import the removed Python transient classes or invoke their `start`, `settle`, media, or auto-save APIs. Use [architecture](architecture.md) and the actual Rust modules to determine which primitives exist. The original Python behavior and its historical CI evidence remain available in prior repository revisions; they are not proof that the current Rust crate enforces the same policy.
