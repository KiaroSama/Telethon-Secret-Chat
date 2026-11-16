# Transient payloads and protected keys

Use `custody::TransientStorage` over `ProtectedFileStorage`, with explicit
`TransientLimits`. `TransientSecretChatManager<S,T>` is the manager type alias.
The manager uses the application's original client and authorization. Disabling
the ordinary history cache alone does not select this policy.

The protected file stores keys, counters, DH/rekey/control metadata and suspension
state. Ordinary payloads, outgoing ciphertext, gap bodies, mailbox content,
settings carrying content and saved-history preferences are rejected before
commit. Unknown fields and invalid pending-create shapes also refuse. Existing
ordinary stores are refused even when empty; no implicit conversion runs.

Live payload obligations remain in bounded RAM. Defaults are 64 chats, 512
records, 8 MiB accounted bytes, 262144 bytes per frame, 300 seconds retained age,
15 seconds per operation, 5 seconds cleanup, eight tasks and three attempts per
original retained frame. Validate custom limits; NaN/infinite/nonpositive values
are refused. External inputs/reservations share the global budget and retain
original ages until their owners settle.

Stop, restart, payload loss, deadline, failed callback or exhausted budgets suspend
continuity. Keys/counters remain, payload buffers are scrubbed from the protected
projection, and reconstructed chats cannot resume a sequence whose frames were
lost. A failed suspension write still publishes safe RAM suspension and refuses
participation; recovery can only persist the normalized full-stop projection.
No Noop/Delete replacement is fabricated to cover a lost ordinary frame.

`set_accepting_requests(false)` refuses new intake while existing healthy chats
continue. `is_suspended(id)` inspects local refusal. Explicit reads and memory-only
media receipt are supported while live; saving/exporting ordinary content is
refused. Auto-save and runtime message history remain off for this mode.

Accounting is not a process-RSS/secure-erasure guarantee. Caller copies, SDK
serialization/queues, swap, crash dumps and external logs are outside addon
control. A queued RPC may execute remotely after its future is dropped. No new
session, sign-in, device change or SDK queue-ack interception is performed by this
mode. Use [strong ingress](strong-ingress.md) for pre-ack encrypted receipts, or
[ordinary durable mode](rust-manager.md) when payload restart continuity is needed.
