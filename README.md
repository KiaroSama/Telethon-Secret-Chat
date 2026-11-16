# Kiaro Secret Chat · Rust

Telegram MTProto 2.0 secret chats for an application-owned
[grammers](https://codeberg.org/Lonami/grammers) client. Rust 2024 crate:
`kiaro-secret-chat` (`kiaro_secret_chat` in code), version `0.3.0-alpha.1`.

[![Rust build and tests](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/rust.yml/badge.svg?branch=rewrite%2Frust-grammers)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/rust.yml?query=branch%3Arewrite%2Frust-grammers)
[![Rust source distribution](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/package-validation.yml/badge.svg?branch=rewrite%2Frust-grammers)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/package-validation.yml?query=branch%3Arewrite%2Frust-grammers)
[![CodeQL Rust](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/codeql.yml/badge.svg?branch=rewrite%2Frust-grammers)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/codeql.yml?query=branch%3Arewrite%2Frust-grammers)
[![Secret scan](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/gitleaks.yml/badge.svg?branch=rewrite%2Frust-grammers)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/gitleaks.yml?query=branch%3Arewrite%2Frust-grammers)
[![Rust 1.99.1](https://img.shields.io/badge/Rust-1.99.1-000000?logo=rust)](rust-toolchain.toml)
[![Linux · macOS · Windows](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)](.github/workflows/rust.yml)
[![x86_64 · ARM64](https://img.shields.io/badge/architecture-x86__64%20%7C%20ARM64-blue)](docs/rust-ci.md)
[![GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)

The rewrite is reviewed in [PR #33](https://github.com/KiaroSama/Telethon-Secret-Chat/pull/33)
without merging. Badges follow its branch. GitHub's default-branch commit indicator
continues to follow `main`; assess the checks on the PR's latest commit.

## Features

- Managed create/accept/close/delete, ordered text and service messages, durable
  handshakes, delivery mailboxes and exact ciphertext retries.
- Automatic/explicit PFS rekey, signed exchange-ID collisions, previous-key
  retirement, fingerprints and stable first-key visualization.
- Eight media kinds, streaming encryption, references, forwarding without
  re-upload, atomic downloads, saved copies and Desktop-style HTML/JSON export.
- Explicit memory/file storage, bounded durable custody, transient payloads with
  protected keys and encrypted commit-before-ack ingress through the SDK seam.
- Typed callbacks, owned asynchronous tasks, cancellation-aware lifecycle and
  diagnostics that exclude plaintext and keys.

Implementation, tools, schema generation and tests are Rust. The exported viewer
retains static HTML/CSS/JavaScript assets as data with their original notices;
there is no JavaScript build toolchain.

## Install from Git

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Telethon-Secret-Chat", branch = "rewrite/rust-grammers" }
```

Pin a reviewed commit with `rev` for reproducible consumption. This alpha is a
source/Git distribution (`publish = false`), not a crates.io release. Use the
crate's `grammers_client`, `grammers_session`, `grammers_mtsender` and
`grammers_tl_types` re-exports to share one SDK type graph. The three bundled SDK
crates resolve transitively; no consumer `[patch]` or installed-SDK mutation is
needed. See [migration and SDK provenance](docs/rust-migration.md).

## Attach to the existing client

```rust
use kiaro_secret_chat::{
    error::Result,
    grammers_client::Client,
    manager::{Options, SecretChatManager},
    storage::FileStorage,
    transport::GrammersTransport,
};

async fn attach(client: Client)
    -> Result<SecretChatManager<FileStorage, GrammersTransport>>
{
    let manager = SecretChatManager::verified(
        GrammersTransport::new(client),
        FileStorage::open("private/secret-chats.json")?,
        Options::default(),
    ).await?;
    manager.start().await?;
    Ok(manager)
}
```

The application owns its client, authorization, runner and update receiver. Pass
updates from that same receiver to `manager.handle_update(&update).await`. Resolve
an `InputUser` with the existing client, then call `create(peer).await`. The peer
must accept before messaging. The manager creates no second session/sign-in flow.

`send_message(id, text, None, None).await` parses Markdown; use
`Some(Vec::new())` for literal text. `Error::SendPending` identifies an uncertain
send; `retry_pending(id).await` reuses the persisted ciphertext and random ID.
`stop().await` settles owned tasks and releases storage. The caller controls its
own pool. Ordinary file-backed mode defaults auto-save on unless a saved setting
disables it; use `stop_auto_save_secret_chats().await` when unwanted.

## Choose a custody mode

| Mode | Held content | Restart | Server acknowledgement |
| --- | --- | --- | --- |
| `MemoryStorage` | RAM | Lost on exit | Normal SDK stream |
| `FileStorage` | Keys/mailbox/gaps/exact outbox on disk | Replays committed work | Normal SDK stream |
| `CustodyStorage` | Bounded durable obligations | Preserves paused work | Normal SDK stream |
| `TransientStorage` + `ProtectedFileStorage` | Payload in bounded RAM; keys/control on disk | Suspends continuity; preserves keys | Normal SDK stream |
| `DurableIngress` + `StrongSecretChatManager` | Encrypted full receipts and protocol state | Restores SDK checkpoint and held input | Fenced until commit and same-client proof |

See [bounded](docs/bounded-mode.md), [transient](docs/transient-mode.md) and
[strong ingress](docs/strong-ingress.md). Ordinary callbacks occur after SDK
processing and cannot establish commit-before-server-ack custody.

## Documentation and verification

- [Managed API, callbacks, media, lifecycle](docs/rust-manager.md)
- [Exporter](docs/rust-export.md), [architecture](docs/architecture.md), [protocol](docs/protocol-reference.md)
- [Primitives](docs/rust-primitives.md), [rekey](docs/rust-rekey.md), [security audits](docs/rust-security-audits.md)
- [Migration](docs/rust-migration.md), [case ledger](migration/README.md), [opt-in live tier](docs/rust-live-tests.md)

```sh
cargo fetch --locked
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin migration-audit -- --require-complete
```

CI always builds ARM64 and x86_64 natively on Linux/macOS/Windows. Nine legs
use Rust 1.99.1, with a separate MSRV 1.95 compatibility check, debug/release
and portable AES. The evidence verifier compares
every identity, result and summary with the 1851-case inventory. Missing/extra
cases, filtered runs and undeclared ignores fail it. Property, stateful, oracle,
lifecycle, installed-Git consumer and SDK boundary scenarios retain their roles.
Ten live cases stay explicitly ignored; Windows additionally ignores three Unix
filesystem cases. CI uses no real Telegram credentials.

## Compatibility and limits

The Rust API and format are breaking changes from the Python package. Native
format 1 accepts earlier optional native fields but does not import Python stores
or Telethon sessions. Incompatible data refuses participation without conversion.
Keep old state with its matching implementation; see [cutover](docs/design/cutover-history.md).

Protect state folders/backups. Unix files are owner-only; Windows operators must
supply owner-only ACLs. File data is flushed before replacement; Unix directory
fsync has a separate contract. Content budgets are not process-RSS limits.
Big-integer DH is not constant time, and zeroizing wrappers do not erase every
allocator/OS copy. AES-IGE file fingerprints bind keys, not file integrity.

Offline tests use independent vectors and a burned official-client capture. A
green offline matrix does not claim a real-account interoperability run.
[`telegram-mcp`](https://github.com/KiaroSama/telegram-mcp) is a future Rust consumer;
this PR does not connect or modify it. GPL-3.0-or-later: retain [LICENSE](LICENSE),
[NOTICE](NOTICE) and [SDK notices](vendor/grammers/README.md).
