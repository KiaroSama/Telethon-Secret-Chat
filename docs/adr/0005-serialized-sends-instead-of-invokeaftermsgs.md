# 0005. Serialize sends around durable identities

Status: accepted; carried into the Rust rewrite.

Per-chat outbound ordering serializes reserve/commit/RPC progression. The retained sequence, random ID, original frame and file descriptor persist before network I/O. Retry uses the same identity after uncertain completion or restart. The library does not chain fragile sender-internal message IDs through invokeAfterMsgs. Generation checks prevent late results from overwriting a new run; cancellation cannot retract an RPC already queued remotely.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
