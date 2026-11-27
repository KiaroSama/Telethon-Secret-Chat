# Bounded custody mode

`Grammers Secret Chat` implements bounded custody through [custody.rs](../src/custody.rs) (`CustodyRecord`, `CustodyStore`, `CustodyLimits`) and [runtime_modes.rs](../src/runtime_modes.rs) (`BoundedSecretChatManager`).

## Data model and limits

`CustodyLimits` bounds retained message count and aggregate bytes:

| Field | Default | Hard ceiling |
| --- | ---: | ---: |
| `max_retained_messages` | 512 | 4096 |
| `max_stored_bytes` | 4 MiB | 64 MiB |

Both limits must be positive. Reaching count or byte capacity preserves all retained payloads and pauses the chat instead of evicting pending obligations.

## Owner binding, pause, and resume

- `CustodyStore::open_durable(path, key)` binds to the first authenticated owner identity (PR #51 R04) so another authorization cannot mix or overwrite records.
- `BoundedSecretChatManager` wraps `PersistentSecretChatManager` and `CustodyStore`, exposing `pause_chat`, `resume_chat`, `sweep_custody`, and `custody_status`.
- Pausing preserves keys and pending queues locally without remote discard. Sweeps pause over-budget chats without eviction or auto-resume.
- `resume_chat` refuses continuation if an irreversible missed-ingress pause reason is recorded on the custody record.

## Persistence boundaries

All writes use exclusive staging files, flushing, atomic replacement without an unlink gap, and cooperative empty-lockfile verification (PR #51 R05). Encrypted custody files hold retained payloads; callers must protect parent directory permissions and store encryption keys separately.
