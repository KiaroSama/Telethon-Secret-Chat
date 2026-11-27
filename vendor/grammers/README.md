# Pinned Rust SDK supplement

These three Rust crates come from official grammers revision
`11e13a989a754f9cf01a798480a2e20ce9e5b1fd`. `upstream-hashes.json` records the
SHA-256 of each original Rust source file before local changes and of the Layer 234
TL schema used by the pinned Git dependency. Each crate retains
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

The client also exposes an opaque home-DC authorization-key tag. The secret-chat
adapter uses this tag to bind state to the actual login key, including when the
current session has a zero session-list management hash. Local key replacement,
key removal, and home-DC changes are rejected before adapter RPC verification;
the user identity is still verified with Telegram. The tag does not expose the
authorization key and is not itself proof that a user is signed in.

The client uses Jiff 1.0.3 and its renamed `parse_zoned_datetime` API. This local
dependency adaptation preserves named-zone precedence over a conflicting offset,
compatible daylight-saving disambiguation, and UTC fallback for date-times
without a zone or offset. Parser regressions cover these behaviors and invalid
date-time rejection.

`local-patches.json` records the exact distributed source hashes and explicitly
lists patched upstream files and added supplement sources. The original hashes
in `upstream-hashes.json` remain the unmodified official reference; patched files
are not represented as new upstream captures. `prepare-sdk` verifies those
official inputs before reproducing this project's complete patched source graph.

Direct path dependencies keep the same SDK type graph for Git consumers; Cargo
patches do not propagate from a dependency to its consumer. Use the SDK crates
re-exported by `kiaro_secret_chat`. The TL layer is 234. This source release
includes API corrections: absent media IDs and missing refetched
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
