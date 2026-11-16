# Pinned Rust SDK supplement

These three Rust crates come from official grammers revision
`e61cf8585e1e9969ab81eda9fb4725a1500e2d86`. `upstream-hashes.json` records the
SHA-256 of each original Rust source file before local changes. Each crate retains
the upstream MIT and Apache-2.0 license texts and source notices.

`grammers-session` adds the bounded MessageBoxes checkpoint and complete
live/output codecs. The checkpoint retains gap bodies, deadlines, difference
ownership and account/channel counters. Raw update bodies were removed from
MessageBoxes tracing. `grammers-mtsender` adds one pre-start owner shared by every
original handle clone, request/result permits, durable live/difference capture,
restore/replay and notification of caller-issued difference receipts. Successful
custody commit precedes observed-machine replacement and delivery. Default pools
have no hook and preserve the ordinary SDK path.

`grammers-client` source is unchanged. It is included to make its pool/session
types refer to the same two supplemented crates when an application consumes
this repository as a Git dependency. Cargo patches do not propagate from a
dependency to its consumer. Direct path dependencies avoid that hidden
prerequisite. Use the SDK crates re-exported by `kiaro_secret_chat` for one
canonical type graph. The client retains the official public API and TL layer.

The manifests have standalone version declarations and pinned official Git
references for the remaining SDK crates; upstream workspace-only development
configuration and examples are excluded. There is no installed-package rewrite,
runtime monkeypatch or C/Python ingress helper. The root crate is a source/Git
distribution and is not published to crates.io.

The supplement remains under verification in the migration PR. Its presence does
not establish live Telegram interoperability or complete 1851-case parity.
