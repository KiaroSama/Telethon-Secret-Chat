# Rust migration to grammers

**Status: incomplete, not a production replacement.** The existing Python code is
the behavioral reference until all 1851 collected cases and all features have
verified Rust equivalents. Do not merge the migration PR or switch a running
consumer based only on passing Rust component tests.

## Dependency and ownership

The new `kiaro-secret-chat` crate uses Rust 2024, an MSRV of 1.95 and the pinned
Rust 1.100 toolchain for development. Every grammers dependency comes from
`https://codeberg.org/Lonami/grammers.git` at
`e61cf8585e1e9969ab81eda9fb4725a1500e2d86` (client/crypto/generator/types 0.12.0,
parser 2.0.0). Cargo.lock pins the transitive graph. Upstream crates are not
vendored and no installed dependency is patched at runtime.

`GrammersTransport::new(client.clone())` reuses the application's existing
`grammers_client::Client`. It does not create a client, log in, import another
session, add a device, or send a Telegram request until an async operation is
explicitly called. Callers retain ownership of the sender pool and update stream.

The adapter provides typed DH, request/accept/discard and sendEncrypted* RPCs.
`Protocol` implements synchronous state transitions. It commits retained bytes,
random IDs and counters before returning a `SendEffect`. Execute that effect on
the same transport and then call `confirm_sent`; on an uncertain send, retry the
retained effect rather than calling `send_message` again.

The explicit authorization binding passed to `Protocol::open` must be a stable
SHA-256 identifier derived by the caller from the original account/authorization.
It is compared with persisted state. The current adapter does **not** independently
prove that a caller-supplied binding belongs to the supplied client. Integration
must add that proof before exposing an automatic manager.

## Components currently implemented

- MTProto 2 KDF, constant-time message-key comparison, padding and frame checks,
  explicit initiator/responder directions and zero-key-ID refusal.
- Checked 2048-bit safe-prime DH and public-value bounds, secure random exponents,
  signed key fingerprints and the original key visualization.
- All 90 secret-schema constructors generated at build time in Rust. Historical
  constructor names receive Rust-only suffixes; wire IDs and families stay intact.
- A bounded schema preflight rejects invalid UTF-8, excessive depth, truncated
  fields, invalid vector lengths, wrong result families and trailing bytes before
  calling grammers' deserializer. The public low-level generated `Deserializable`
  trait itself remains an unchecked primitive: use the validated entry points.
- Mirrored sequence counters, bounded gap recovery, retained exact-byte resends,
  consumer-acknowledged durable delivery records and explicit per-chat pause.
- Manual rekey request/accept/commit/abort handling, signed-ID collision handling,
  the previous-key window and stable original key visualization. The automatic
  trigger predicate exists; automatic scheduling is still pending.
- Explicit memory/file state backends, atomic whole-state replacement, a writer
  lock, file fsync, Unix directory fsync and refusal after uncertain replacement.
- Separate file keys and fingerprints, bounded AES-IGE streaming and atomic file
  saving that preserves an existing destination on truncated input.
- Representable text-entity conversion and bounded Ogg music/voice classification.

The crate is intentionally not a drop-in Python API. Full managed lifecycle,
subscription ownership, callback handling and fault recovery are acceptance work,
not implied by these component APIs.

## Verification

```sh
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin migration-audit
cargo run --locked --bin migration-audit -- --require-complete
```

The last command must fail until the case ledger is complete. Rust CI always
builds before tests and covers Linux, macOS and Windows, debug/release, the MSRV
and a forced portable AES backend. Existing Python qualification jobs stay active
during the migration. New Rust test count is not substituted for the old count.

Synthetic schema vectors are emitted by the unchanged legacy encoder, not by the
new Rust serializer. Two KDF vectors come from grammers' own independent tests.
The existing official-client throwaway-chat capture is replayed offline. No live
Telegram interoperability is claimed for this rewrite, and no real account is
used by CI.

## Blocking acceptance work

1. Map and execute every original case individually, keeping 1851 cases and their
   types. Port property/state-machine, regression, exporter, package, Windows
   launcher, installed-package, offline oracle and explicitly gated live tiers.
2. Complete the managed create/accept/restart lifecycle, early terminal updates,
   lost request identities, automatic rekey and service-event contracts. Validate
   actual account/authorization binding rather than accepting a supplied string.
3. Port the eight media kinds, metadata/reference/forwarding APIs, media upload
   and download, saved copies and HTML/JSON exporter, including Qt ICC/thumbnail
   and asset oracles. Existing content must remain readable during cutover.
4. Port transient and bounded modes with all refusal, budgets, relay and restart
   invariants. A pause flag alone is not bounded durable custody.
5. Implement and prove protected pre-ack ingress for both live and difference
   envelopes, original client identity, durable baseline, transactional frontiers,
   reconnect, backpressure and cancellation. Stock Raw delivery alone cannot
   establish this guarantee. `require_strong_ingress` currently fails explicitly.
6. Provide versioned, recoverable migration of existing Python stores. Current
   Rust format version 1 refuses incompatible input and does not migrate it.
7. Once parity is verified, remove Python implementations/tooling/lockfiles and
   migrate all legacy workflows, packaging, Dependabot, docs and badges. Replace
   CodeQL Python configuration with a supported Rust security-analysis setup;
   do not label an unsupported language setting as a successful CodeQL scan.
8. Consolidate outstanding dependency branches and review every installed bot's
   final-head feedback. Publish the complete result in this one PR without merging.

`telegram-mcp` is a future consumer only. No consumer file, session, account or
configuration is changed by this PR. Connect it only after the owner's separate
Rust migration is ready and the owner requests the final integration check.

## Practical limits

State files contain key material and normal-mode payloads. Protect the parent
folder and backups. File permissions are 0600 on Unix; Windows requires an
owner-only ACL supplied by the operator. The current Windows implementation
flushes file data before atomic replacement; it does not claim Unix-style
directory fsync. The backend is single-writer and rejects a concurrent writer.

BigUint modular exponentiation is not constant-time and its internal temporary
allocations are not guaranteed to be zeroized. Shared-key and private-exponent
wrappers are zeroized on drop, which does not imply secure erasure of all copies
or storage media. AES-IGE file encryption does not authenticate file contents.

## Sources checked

- [grammers source and 0.12.0 release](https://codeberg.org/Lonami/grammers/commit/e61cf8585e1e9969ab81eda9fb4725a1500e2d86)
- [Telegram end-to-end specification](https://core.telegram.org/api/end-to-end)
- [Secret-chat schema](https://core.telegram.org/schema/end-to-end)
- [TDLib SecretChatActor](https://github.com/tdlib/td/blob/c8828d50f5eac2fc7089ce1089d7ce9095c61b5e/td/telegram/SecretChatActor.cpp)
- [TDLib zero-key-ID fix](https://github.com/tdlib/td/commit/d8cc525a847c178a67b9cc7172fb1f45f0b153a2)
- [Archived original implementation](https://github.com/painor/telethon-secret-chat/tree/32e7585ba4f5dbb2c6f025047c11fa52cf55d071)
