# Transient mode

`TransientSecretChatManager` in [runtime_modes/mod.rs](../src/runtime_modes/mod.rs) enforces a RAM-only payload boundary backed by `ProtectedFileStorage` / `SecretChatStore` in [storage_backend/mod.rs](../src/storage_backend/mod.rs).

## Core guarantees

- Ordinary and service message payloads remain in RAM only; `CustodyStore::save_record` refuses disk persistence for records marked `CustodyMode::Transient` or `CustodyBackend::MemoryOnly`.
- Outgoing messages select the protocol's current TTL under the chat-state lock when their sequence is reserved, after older pending sends drain. An authenticated peer TTL update received while a send waits therefore applies to that new message. A retry retains the original ciphertext and its original TTL.
- The wrapper does not retain a second, inaccessible RAM event queue. Its returned receive batches remain caller-owned; the public RAM `SecretChatManager` continues to expose all event categories through `drain_events`.
- On process restart (`open_after_restart`), previously active transient chats are recorded as permanently suspended and cannot silently resume with missing volatile payloads.
- Subsequent sends or receives on a permanently suspended transient chat fail closed with `SecretChatError::Protocol("transient chat permanently suspended after restart")` until explicitly closed or forgotten.

## Boundaries

In-memory custody reduces disk exposure and zeroizes key material on drop, but cannot guarantee against OS swap, crash dumps, or copies retained by caller code or external transports. Callers remain responsible for authorization, media/archive decisions, and transport-level `qts` acknowledgement.
