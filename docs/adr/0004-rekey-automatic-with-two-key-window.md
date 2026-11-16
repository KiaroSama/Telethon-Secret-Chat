# 0004. Automatic rekey and previous-key window

Status: accepted; carried into the Rust rewrite.

Rekey checks run before content and service sends. A due message-count/age trigger starts at most one exchange. Request/accept/commit/abort state persists transactionally; signed exchange IDs resolve simultaneous requests. The receiver accepts the justified current, pending accepted or previous key. New-key adoption queues a confirmation Noop in the same commit, including while a receive gap is open. Confirmation retires the previous key. The first-key visualization remains stable. A legacy native record lacking DH can use its existing key but closes with a fixed reason when rekey becomes necessary.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
