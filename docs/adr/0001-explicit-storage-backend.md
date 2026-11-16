# 0001. Explicit storage backend

Status: accepted; carried into the Rust rewrite.

The manager requires an explicit Storage backend. MemoryStorage is intentionally volatile; FileStorage atomically persists the whole validated State. Protocol drafts include counters, keys, mailboxes, gaps and retained ciphertext in one commit. Effects become visible only after a successful commit. A failed/uncertain backend never publishes an advanced RAM draft. Choose protected/bounded/ingress wrappers explicitly; no hidden session database is modified.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
