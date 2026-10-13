# Bounded durable integration

`BoundedSecretChatManager` is an explicit opt-in over the existing durable protocol manager.
It uses the caller's existing connected Telethon client and backend, never a second login,
session export or consumer activation. Default `SecretChatManager` and the RAM-only
`TransientSecretChatManager` remain unchanged.

The separate [strong-ingress mode](strong-ingress.md) targets the earlier
Telethon queue-acknowledgment gap. It is not activated by this mode;
its public prerequisite configuration validates protected attachment before connection. The guarantees
and limitations below remain the bounded manager's supported contract.

## Construction and archive policy

```python
from telethon_secret_chat import BoundedSecretChatManager, CustodyLimits

manager = BoundedSecretChatManager(
    client,
    dedicated_store,
    limits=CustodyLimits(records=512, bytes=8 * 1024 * 1024, age_seconds=3600),
)
await manager.stop_auto_save_secret_chats()  # mandatory BEFORE start for no local archive
await manager.start()
```

The example budgets are application policy, not a server retention guarantee. Explicit
FileStorage still defaults auto-save ON absent a stored preference. `auto_save_folder=None`
is not an OFF switch. MemoryStorage is explicitly volatile. A custom backend must implement
transactional durable `load_setting`/`save_setting` as well as the full existing synchronous
transaction contract. The base process-only settings fallback is rejected. Use a fresh dedicated
store; existing protocol records without complete bounded age/control metadata are refused,
not silently migrated or given fresh ages.

The store holds keys, original ciphertext/RPC identities, gap bodies, incoming mailbox and
relay obligations, including plaintext and file keys. This is necessary temporary protocol
custody, **not payload-free storage**. FileStorage is not encrypted at rest. Consumers own
store encryption, account/auth/DC binding, exclusive ownership and exclusions from backups,
packages and deployment. Bounded media receipt writes no local media file; OS swap, crash
dumps, returned bytes and external client/application logging are outside that claim.

## Public interfaces

- `CustodyLimits(records, bytes, age_seconds, *, chats=64, frame_bytes=262144,
  operation_seconds=15, tasks=8)` validates positive finite values. Hard ceilings: records4096,
  bytes64MiB, chats64, frame1MiB, age30days, operation60seconds, tasks64. Serialized custody
  bytes are not RSS. Records count retained outbox, gaps, mailbox and consumer obligations.
- `pause_chat(chat_id)` persists local pause without deleting or remote-discarding records.
  `pause_status(chat_id)` reports paused/resumable/reason/records/bytes without payload.
- `check_custody()` checks capacity/operational age now. Every admitted operation checks too;
  there is no timer-based automatic eviction. While idle, the application calls this sweep.
- `resume_chat(chat_id)` is explicit, verifies custody and refuses missed required ingress.
  A paused conversation does not block unrelated healthy conversations.
- Optional synchronous `admission(chat_id, operation) -> bool` receives no payload. False,
  exception or a non-bool refuses before an effect. `create` uses chat_id=None because the
  server has not assigned it. Admission does not authorize acceptance or user read.
- `pending_deliveries(chat_id)` returns detached `RelayDelivery` values. `seq_no` is the
  **wire sequence**, identical to `delivery.message.seq_no`, stable across reconstruction.
  `acknowledge_delivery(chat_id, seq_no)` releases only that consumer-committed obligation;
  unknown identities raise ValueError. It sends no read receipt or RPC.
- `receive_file(message, *, max_bytes)` returns decoded bytes in bounded RAM. The declared
  size is checked before download; actual writes cannot exceed padded declared size;
  fingerprint and ciphertext-length checks use the existing file implementation. Finite
  operation and shared active-media limits apply, and cancellation closes the RAM buffer.
  `save_file` remains the existing explicit disk-writing API, not the no-archive path.

## Effect and failure boundary

Admission precedes receive decryption, establishment, send preparation, retries, rekey,
startup recovery and upload. Original outgoing bytes/identity commit before RPC. A pause
cannot retract an already-issued RPC; result bookkeeping may reconcile the existing record,
never synthesize a fresh send. A persisted DH secret can reconcile uncertain acceptance;
a paused acceptance result must not install a key or emit ready. Capacity/age failures retain
previous bounded custody and warn only for the affected chat. No automatic deletion, close,
silent eviction or physical-erasure guarantee is provided. In this mode `close` becomes a
preserving nonresumable local stop; destructive chat deletion/forget APIs refuse. A peer
history-delete or fatal protocol failure stops participation with retained custody rather
than silently deleting consumer obligations. Default manager deletion semantics are unchanged.

Async event callbacks remain notifications, not settlement. Ordinary mailbox-to-relay
projection and mailbox removal share one backend transaction before callback dispatch.
Failed or absent callbacks leave polling available. Protocol service actions can proceed
without waiting for ordinary consumer acknowledgment, subject to the same custody budget.
Callback concurrency/deadline limits reduce library-owned work; arbitrary caller callbacks
and third-party operations that suppress cancellation cannot be forcibly terminated by Python.

## Cloud relay idempotency belongs to the consumer

Persist a downstream Telegram random_id with the stable `(chat_id, wire_seq_no)` obligation
before issuing a cloud send. Reuse that identity after uncertain results or reconcile the
existing cloud message. Never retry an unknown outcome with a fresh identity; acknowledge
only after the consumer has durably committed its result/mapping. This library makes no
exactly-once cloud delivery guarantee and performs no forum/topic/read/reply integration.
Read receipts require the consumer's authenticated explicit action with mapped IDs. Replies
use the chat's existing TTL; no implicit timer/read/device change is made here.

## Transport qts is independent

Telethon advances update state and schedules transport acknowledgments before Raw callbacks.
A subsequent `updates.getDifference` can implicitly acknowledge its INPUT qts; difference
messages can have no individual qts. This library cannot roll that back through public event
registration. Successfully committed post-Raw custody survives reconstruction; rejected
updates or crashes before that commit are **not promised recoverable**. A refused required
inbound update makes local continuation nonresumable. Keys/counters and transaction rollback
alone prove neither server retention nor lossless replay. Telegram may remove acknowledged
or older-than-seven-day secret-queue events.

Sources: [Telegram E2E](https://core.telegram.org/api/end-to-end),
[sequence/replay](https://core.telegram.org/api/end-to-end/seq_no),
[PFS](https://core.telegram.org/api/end-to-end/pfs),
[Telethon1.45 release](https://pypi.org/project/Telethon/1.45.0/),
[TDLib retained-send source](https://github.com/tdlib/td/blob/c15d3f5a5de6e3ba5839822c451152e5e18bb700/td/telegram/SecretChatActor.cpp).

## Verification scope

Public seam fixtures exercise pause isolation, preserved custody/restart/original retry,
consumer acknowledgment, admission rejection and capped RAM media without live accounts.
Wire lifecycle simulations are not independent cipher interoperability evidence; existing
vectors and burned official-client fixtures cover the unchanged cryptography. Full
Linux/Windows matrix and installed-wheel evidence must be checked on the final revision.
A library handoff does not implement consumer storage protection or validate actual cloud
forum/read/reply/media flows; those require separately authorized consumer work.
