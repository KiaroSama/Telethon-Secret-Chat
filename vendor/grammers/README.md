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

All three crates also carry local correctness fixes found during the migration
review. The client corrections cover request/result association, pagination,
bounded file transfer, cancellation-safe update caching, peer/media boundaries,
and HTML/Markdown parsing and generation. Session corrections preserve counters,
deadline restoration, bulk peer caching and atomic optional SQLite operations.
The sender preserves typed error causes and redacts proxy credentials.

`local-patches.json` records the exact distributed source hashes and explicitly
lists patched upstream files and added supplement sources. The original hashes
in `upstream-hashes.json` remain the unmodified official reference; patched files
are not represented as new upstream captures. `prepare-sdk` verifies those
official inputs before reproducing this project's complete patched source graph.

Direct path dependencies keep the same SDK type graph for Git consumers; Cargo
patches do not propagate from a dependency to its consumer. Use the SDK crates
re-exported by `kiaro_secret_chat`. The TL layer remains 233. This source alpha
includes deliberate API corrections: absent media IDs and missing refetched
messages use `Option`, participants retain non-user peers, and unavailable dialog
references and callback messages return explicit absence/errors. See
`docs/rust-export.md` in the root repository for the precise API changes.

The manifests have standalone version declarations and pinned official Git
references for the remaining SDK crates; upstream workspace-only development
configuration and examples are excluded. There is no installed-package rewrite,
runtime monkeypatch or C/Python ingress helper. The root crate is a source/Git
distribution and is not published to crates.io.

The supplement remains under verification in the migration PR. Its presence does
not establish live Telegram interoperability or complete 1851-case parity.
