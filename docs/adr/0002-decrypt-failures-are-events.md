# 0002. Decrypt failures are fixed events

Status: accepted; carried into the Rust rewrite.

The update entry point emits fixed DecryptFailed events and does not format raw cryptographic/parser errors. Authentication/outer-wrapper failures do not close a healthy chat. Fresh authenticated inner-family, parity, decreasing-layer, impossible echo/gap or unsatisfiable resend violations are fatal in ordinary durable mode. Bounded/protected modes preserve keys and refuse continuity according to their policy. Old replays are dropped without inspecting their inner content. Terminal state commits before the close callback.

See [architecture](../architecture.md), [managed API](../rust-manager.md) and
[mode selection](../../README.md#choose-a-custody-mode).
