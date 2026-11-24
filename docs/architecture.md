# Rust architecture

The application owns one grammers client, authorization, sender-pool runner and
update receiver. The library never signs in or constructs another client.
`manager::SecretChatManager<S,T>` verifies binding and owns secret-chat work;
`Protocol<S>` performs synchronous validated state transitions.

| Component | Responsibility |
| --- | --- |
| `crypto`, `dh`, `framing`, `wire`, generated `types`/`enums` | MTProto 2, safe-prime checks, typed strict TL and sequence headers |
| [`aes_ige`](../src/aes_ige.rs) | In-place AES-256-IGE, shared by MTProto frames and file transfers |
| `chat`, `sequence`, `rekey`, `actions` | Validated records, gaps, original-byte resend, PFS and services |
| `protocol` | Clone/validate/commit transactions and effects after commit |
| `storage`, `custody` | Memory/file state, writer ownership, bounded/protected payload policy |
| `manager` | Handshakes, sends, ordered mailbox delivery, callbacks, lifecycle |
| `media`, `files`, `ogg`, `entities` | Media metadata/reference/forwarding, streaming AES-IGE, entity conversion |
| `autosave`, `export` | Saved copies and Desktop-style HTML/JSON with embedded assets |
| `ingress`, bundled SDK supplement | Original-client proof, whole receipt/checkpoint custody, pre-ack fencing |

## AES-256-IGE

`src/aes_ige.rs` uses the safe RustCrypto `aes` 0.9 API and dispatches to the
selected backend once per buffer. The crate and module enforce
`#![forbid(unsafe_code)]`. `IgeEncryptor` and `IgeDecryptor` keep the expanded
AES-256 key and rolling IGE IV across file chunks. The `aes` dependency's
`zeroize` feature clears its key schedule on drop, and the module zeros the IV.

## Commit and effect ordering

`Protocol` clones a draft, validates it, commits it and only then publishes RAM
and returns effects. File replacement flushes staging bytes first. Any uncertain
replacement poisons protected/bounded/ingress participation. Ordinary directory
sync failure reports a fixed warning after a complete replacement. Shared keys
and private records have no payload-bearing Debug implementation.

Outgoing random IDs, sequence numbers, ciphertext and file metadata are retained
before RPC. Uncertain sends remain pending and retry identical bytes. Peer echo
counters prune acknowledged retention. Low-level callers confirm the actual
completed `SendEffect` with `confirm_sent(&effect)`; a stale response cannot mark
rewritten withdrawal ciphertext as sent. Ordinary messages and services share one
ordered durable mailbox. Service effects commit before callbacks; a stored applied
marker prevents replaying the effect when callback delivery retries.

Authenticated Resend is interpreted immediately, including across a receive gap.
Its retained action becomes Noop so replay cannot answer it twice. A pending
answer remains retryable even when later echo pruning removes old retention.
Replays below the receive counter validate their authenticated outer header but
do not inspect their inner content. Fresh inner-family violations close ordinary
chats; outer/decryption failures remain nonfatal diagnostic events.

## Ownership and concurrency

Per-chat incoming/outgoing ordering prevents sequence races. Lifecycle generations
prevent late RPC/upload results from changing a restarted run. Owned tasks and
cancellation cleanup settle before releasing protected storage. Start/stop use a
shared completion so a cancelled caller cannot cancel the lifecycle coordinator.
A callback may await stop without its own task being aborted by that stop.

Finite modes share record/byte/chat/task budgets, original creation times,
operation deadlines and cancellation ownership. A dropped RPC future cannot undo
remote execution already queued in the SDK; retained identities remain necessary.
Returned content, allocator copies and external SDK/OS buffers are outside the
library's content accounting budget.

## Strong ingress

Ordinary post-SDK updates cannot establish pre-ack custody. The bundled SDK
supplement records the complete envelope and exact next MessageBoxes checkpoint
before replacing the observed machine, returning difference results or allowing
an acknowledgement beyond the durable frontier. It attaches before the runner
starts, has one receiver and proves the original client using the same pool.
Protocol application and ingress cursor commit to the same encrypted state.
Read [strong ingress](strong-ingress.md) for the full setup and refusal contract.

Start with the [usage guide](usage-guide.md). To verify a native package, see
[Publisher signatures](SIGNING.md).

See [managed API](rust-manager.md), [primitives](rust-primitives.md),
[mode selection](../README.md#choose-a-custody-mode), [exporter](rust-export.md)
and [migration](rust-migration.md). The schema source is `schema/end-to-end.tl`;
`build.rs` invokes the pinned native grammers generator, without another language.
