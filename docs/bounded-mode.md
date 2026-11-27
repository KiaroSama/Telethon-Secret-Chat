# Bounded custody in the current Rust crate

The current [Rust custody API](../src/custody.rs) provides a bounded payload container and an encrypted snapshot store. It does **not** provide the retired Python `BoundedSecretChatManager`. A custody mode label does not add persistence, admission controls, or safe restart behavior to `manager::SecretChatManager`.

## Data model and limits

`custody::CustodyRecord` stores a chat identifier, caller authorization identity, mode/backend selections, limits, pause state, and retained opaque payload bytes. `CustodyLimits` contains:

| Field | Default | Hard ceiling |
| --- | ---: | ---: |
| `max_retained_messages` | 512 | 4096 |
| `max_stored_bytes` | 4 MiB | 64 MiB |

Both limits must be positive. The encrypted serialized document also has a 64 MiB ceiling; JSON representation and other records consume space, so the payload ceiling does not promise that every maximum-sized record will fit on disk.

`push_payload` refuses a payload larger than its configured byte ceiling. Reaching aggregate count/byte capacity preserves all retained payloads and pauses the record instead of evicting an older obligation. A paused record refuses further pushes. Restored accounting, limits, identity, and pause fields are validated before use.

The container has no protocol sequence identities, retained-RPC metadata, message-age limit, admission callback, task budget, or native media transfer API. It is not a complete retransmission or relay-obligation implementation.

## Pause and resume

`pause(reason)` sets the local pause flag; saving the record persists that state. `resume()` clears the flag in the local object. It does not check Telegram continuity, discover a missing frame, reconcile a rekey exchange, or establish that an old conversation can safely participate.

After a failed or missed required inbound update, the caller must not interpret `resume()` as proof of recovery. Capacity checks still apply after clearing a pause flag. There is no automatically coordinated manager pause or consumer acknowledgement method on `CustodyRecord`.

This is narrower than the historical Python bounded-mode contract, which described protocol-wide admission, stable relay identities, age control, and verified resumability. Those manager features are unavailable in the current Rust implementation.

## Persistence

`CustodyStore::open_durable(path, key)` requires an explicit absolute file path and an existing parent directory. The caller supplies the AES-256-GCM key and protects the directory, including Windows ACLs.

`save_record` accepts persistent records using `CustodyBackend::EncryptedFile`. Records marked `MemoryOnly` or `Transient` refuse disk persistence. A store cannot silently mix or overwrite another authorization's records. `load_record(chat_id, expected_owner_auth_id)` authenticates the document and checks the requested owner.

Updates use a file lock, unique exclusive staging, file flushing, replacement without an unlink gap, and parent-directory flushing on Unix. Failed writes preserve prior records; uncertain post-replacement outcomes require reopening and reconciliation. `cleanup_owned_files` authenticates the store before deletion and leaves unrelated temporary files alone.

The file contains the retained payloads themselves. Encryption does not make it payload-free, prevent rollback to an older valid file, or guarantee physical erasure. Exclude encryption keys and custody files from diagnostic bundles and unintended backups.

## Manager outbox limits are separate

`manager::ManagerLimits` bounds the live manager's retained outbound message count and ciphertext bytes. The manager retains the original ciphertext and random ID before awaiting transport and retries that same transmission after an uncertain outcome. Authenticated peer acknowledgement releases retained sends.

Those limits do not install a `CustodyStore`, persist the outbox, or restore a manager. They also do not bound the event queue, registered-chat map, caller tasks, or third-party transport memory. There is no automatic archive policy to enable or disable because the Python auto-save subsystem is not implemented here.

## Migration notes

Older Rust snapshots silently evicted custody payloads at capacity. Current code refuses and preserves them. Existing callers must handle the error and paused state rather than assuming a rolling history window.

Malformed or inconsistent stored records now fail at reconstruction. Cleanup no longer deletes an unauthenticated file merely because it has the configured filename. Recovery or disposal of such a file is an explicit operator task.

The historical Python mode remains described by earlier repository revisions. Its examples using `BoundedSecretChatManager`, `pending_deliveries`, `acknowledge_delivery`, `receive_file`, and auto-save settings are not instructions for the current Rust crate. See [architecture](architecture.md) for actual component boundaries and [ingress storage](strong-ingress.md) for the separate opaque-record acknowledgement API.
