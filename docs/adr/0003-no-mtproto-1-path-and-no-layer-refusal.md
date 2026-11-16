# 0003. MTProto 2 only

Status: accepted; carried into the Rust rewrite.

crypto implements MTProto 2 only, without downgrade/fallback after a failed decrypt. Outgoing layers are bounded to 73–144. An explicit layer validation helper exists, but ordinary receive does not negotiate a MTProto 1 path for an old peer. Undecryptable frames surface as fixed failures. This follows the previous scope decision; no unauthenticated byte can trigger weaker cryptography.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
