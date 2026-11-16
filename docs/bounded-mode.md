# Bounded durable custody

Wrap an explicit backend in `custody::CustodyStorage::new(backend, binding,
CustodyLimits::new(records, bytes, age_seconds)?)`, then construct the ordinary
verified manager over it. `BoundedSecretChatManager<S,T>` is the corresponding
type alias. The same existing client and transport are reused.

All retained outgoing frames, receive gaps and delivery records share finite
record/byte/age limits. Active media inputs, callbacks, held create identities
and tasks also reserve shared headroom. Persisted identity ages survive restart;
malformed/missing metadata refuses recovery rather than resetting age to now.
Limits are content accounting, not a process-RSS or SDK-buffer guarantee.

Exhaustion pauses participation and preserves required records and keys. There
is no silent eviction followed by continued sequence use. `pause_status(id)`
exposes a fixed reason and continuity status. `pause_chat(id)` pauses explicitly;
`check_custody()` checks capacity/age. `resume_chat(id).await` only resumes intact,
healthy custody and retries original effects. Lost continuity remains refused.
Closing or terminal transport failure preserves bounded obligations and marks
continuation nonresumable; ordinary destructive cleanup is not inherited.

A created chat can still complete remotely after a timeout. Global creation slots
cover held DH/request work and durable pending identities so parallel creates
cannot exceed the chat budget. Uncertain send errors identify the original random
ID; cancellation does not invent a new message or reuse its sequence slot.

Handlers run as owned tasks in finite modes, within their operation/cleanup
policy. No blocking consumer callback is run inline while a protected owner is
held. Shutdown joins cleanup before releasing the backend. An unresolved cleanup
refuses success/restart rather than silently dropping its ownership.

This bounds addon custody after normal SDK delivery. For commit-before-server-ack
receipt custody, use [strong ingress](strong-ingress.md). See [managed API](rust-manager.md)
and [transient mode](transient-mode.md) for different persistence policies.
