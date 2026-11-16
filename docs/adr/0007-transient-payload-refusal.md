# 0007. Transient payload refusal

Status: accepted; carried into the Rust rewrite.

TransientStorage retains ordinary payload obligations only in finite RAM over protected key/control state. Stop/reconstruction, lost payloads, deadlines and exhausted budgets suspend continuity and preserve keys/counters. Content cannot be reconstructed from keys, so no automatic resume or fabricated gap replacement is offered. This sacrifices availability for the explicit no-payload-write policy. Normal SDK queue acknowledgement remains application-owned; strong ingress is a separate policy.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
