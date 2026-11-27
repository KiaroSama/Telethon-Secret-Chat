# Transient mode

`TransientSecretChatManager` in [runtime_modes.rs](../src/runtime_modes.rs) enforces a RAM-only payload boundary backed by `ProtectedFileStorage` / `SecretChatStore` in [storage_backend.rs](../src/storage_backend.rs).

## Core guarantees

- Ordinary and service message payloads remain in RAM only; `CustodyStore::save_record` refuses disk persistence for records marked `CustodyMode::Transient` or `CustodyBackend::MemoryOnly`.
- On process restart (`open_after_restart`), previously active transient chats are recorded as permanently suspended and cannot silently resume with missing volatile payloads.
- Subsequent sends or receives on a permanently suspended transient chat fail closed with `SecretChatError::Protocol("transient chat permanently suspended after restart")` until explicitly closed or forgotten.

## Boundaries

In-memory custody reduces disk exposure and zeroizes key material on drop, but cannot guarantee against OS swap, crash dumps, or copies retained by caller code or external transports. Callers remain responsible for authorization, media/archive decisions, and transport-level `qts` acknowledgement.
