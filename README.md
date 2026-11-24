<div align="center">

# Kiaro Secret Chat · Rust

**Telegram MTProto 2.0 end-to-end encrypted secret chats for a caller-owned grammers client in Rust.**

[![GitHub Actions CI across 64-bit Linux and Windows](https://img.shields.io/badge/CI-GitHub%20Actions-16a34a)](.github/workflows/rust.yml)
[![GNU General Public License v3.0 or later](https://img.shields.io/badge/license-GPL--3.0--or--later-2563eb)](LICENSE)
[![Crate release v0.3.0 with GPG-signed archives](https://img.shields.io/badge/release-v0.3.0-ea580c)](CHANGELOG.md)
[![169 offline release tests and 1,851 mapped parity cases](https://img.shields.io/badge/tests-169%20release%20%7C%201851%20cases-0d9488)](migration/README.md)
[![Rust 1.100.0 measured locally, edition 2024, MSRV 1.95](https://img.shields.io/badge/Rust-1.100.0-8b5cf6)](rust-toolchain.toml)
[![Runs on 64-bit Linux and Windows](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-0891b2)](.github/workflows/rust.yml)
[![Install from Git or GPG-signed release archives](https://img.shields.io/badge/install-Git%20%7C%20release-4f46e5)](#install-from-git)
[![Architecture documentation, protocol reference, and ADRs](https://img.shields.io/badge/docs-architecture%20%7C%20ADRs-0284c7)](docs/architecture.md)
[![Published tree measured via git ls-files is 14.03 MB](https://img.shields.io/badge/repo%20size-14.03%20MB-ca8a04)](Cargo.toml)
[![Built with grammers 0.12.0 on Telegram API layer 233](https://img.shields.io/badge/built%20with-grammers%200.12.0-15803d)](docs/rust-migration.md)
[![Secret chats use MTProto 2.0 layers 73 through 144](https://img.shields.io/badge/protocol-MTProto%202.0%20L73--144-b45309)](docs/protocol-reference.md)
[![Native builds target x64 and arm64 only](https://img.shields.io/badge/arch-x64%20%7C%20arm64-334155)](docs/rust-ci.md)
[![Project crate forbids unsafe Rust code](https://img.shields.io/badge/safety-unsafe%20forbidden-7c3aed)](Cargo.toml)
[![CodeQL static analysis and Gitleaks secret scanning](https://img.shields.io/badge/security-CodeQL%20%7C%20Gitleaks-c2410c)](.github/workflows/codeql.yml)
[![Support donations](https://img.shields.io/badge/Support-donations-d04a9a)](#donate)

</div>

`kiaro-secret-chat` adds Telegram MTProto 2.0 secret chats to your application's
[grammers](https://codeberg.org/Lonami/grammers) client. The crate version is
`0.3.0`; import it as `kiaro_secret_chat` in Rust.

## Features

The manager opens, accepts, closes and deletes secret chats. It orders text and
service messages, saves handshakes and mailboxes in durable modes, and retries
sends with the original ciphertext. It can rekey automatically or on request,
resolve signed exchange-ID collisions, retire previous keys and keep the first
key's visualization stable.

The library handles eight media kinds. It encrypts streams, forwards references
without re-uploading, saves copies and publishes downloads atomically. You can
export a chat as Telegram Desktop-style HTML or JSON. The HTML viewer uses
static assets with their original notices and needs no JavaScript build tools.

Choose a storage mode to control what stays in memory and what survives a restart.
Strong ingress can commit encrypted receipts before the SDK acknowledges them.

## Install from Git

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Telethon-Secret-Chat", branch = "main" }
```

Install Git and Rust before building. The crate supports Rust 1.95 or newer;
this checkout pins 1.100.0 for development. On Windows, the MSVC toolchain needs
the Visual Studio C++ build tools and Windows SDK. On Linux, install a native
C linker such as GCC. Cargo fetches the pinned SDK dependencies from Codeberg and
the JPEG dependency from GitHub.

Use `rev` in place of `branch` to pin a reviewed commit. The crate is available
from Git or a complete source checkout (`publish = false`). Import
`grammers_client`, `grammers_session`, `grammers_mtsender` and `grammers_tl_types`
through the crate to share its SDK types. Cargo resolves the bundled SDK crates
without a consumer `[patch]`. See [migration and SDK provenance](docs/rust-migration.md).

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

Keep the client's existing authorization, runner and update receiver. Pass each
update from that receiver to `manager.handle_update(&update).await`. Resolve an
`InputUser` with the client, then call `create(peer).await`. Wait for the peer to
accept before sending. You do not need a second session or sign-in.

`send_message(id, text, None, None).await` parses Markdown. Pass
`Some(Vec::new())` for literal text. `Error::SendPending` means the send was
committed locally. Call `retry_pending(id).await` to reuse its ciphertext and
random ID. Call `stop().await` to settle the manager's tasks and release storage.
Your application still controls its own pool. In ordinary file-backed mode,
auto-save starts unless a saved setting disables it. To turn it off, call
`stop_auto_save_secret_chats().await`.

## Logging

The library emits `tracing` events through your application's subscriber. Set
up the subscriber to choose where events go; the library does not create a log
file. Diagnostics from the manager leave out message text and keys. Keep any
logs you retain private.

## Choose a custody mode

| Mode | Held content | Restart | Server acknowledgement |
| --- | --- | --- | --- |
| `MemoryStorage` | RAM | Lost on exit | Normal SDK stream |
| `FileStorage` | Keys/mailbox/gaps/exact outbox on disk | Replays committed work | Normal SDK stream |
| `CustodyStorage` | Bounded durable obligations | Preserves paused work | Normal SDK stream |
| `TransientStorage` + `ProtectedFileStorage` | Payload in bounded RAM; keys/control on disk | Suspends continuity; preserves keys | Normal SDK stream |
| `DurableIngress` + `StrongSecretChatManager` | Encrypted full receipts and protocol state | Restores SDK checkpoint and held input | Fenced until commit and same-client proof |

See [bounded storage](docs/bounded-mode.md), [transient storage](docs/transient-mode.md)
and [strong ingress](docs/strong-ingress.md). The SDK processes ordinary updates
before it calls the manager. Use strong ingress if you need to commit a receipt
before server acknowledgement.

## Documentation and verification

The [manager guide](docs/rust-manager.md) covers callbacks, media and shutdown.
See the [export guide](docs/rust-export.md) for saved chats, or the
[architecture](docs/architecture.md) and [decision records](docs/adr/) for the
library's design. The [protocol reference](docs/protocol-reference.md),
[primitives guide](docs/rust-primitives.md), [rekey guide](docs/rust-rekey.md) and
[security audits](docs/rust-security-audits.md) cover the wire and key handling.
For test evidence, see the [migration notes](docs/rust-migration.md),
[case ledger](migration/README.md) and [opt-in live tests](docs/rust-live-tests.md).
See [Publisher signatures](docs/SIGNING.md) to verify native packages.

```sh
cargo fetch --locked
cargo build --locked --all-targets
cargo test --locked --all-targets
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin migration-audit -- --require-complete
```

CI builds natively on Linux and Windows for x64 and arm64. Six legs use Rust
1.100.0, including debug, release and portable AES checks. A seventh tests the
minimum supported Rust version, 1.95. The verifier rejects missing or extra
cases, filtered runs and undeclared ignores against the 1,851-case inventory.
CI uses no Telegram credentials and skips the ten opt-in live cases. Windows
also skips three Unix filesystem tests. The matrix does not cover macOS.

## Compatibility and limits

The Rust API and storage format break compatibility with the Python package.
Native format 1 reads older Rust stores that omit optional fields. The library
rejects Python stores and Telethon sessions. Keep that state with the matching
Python version. See the [cutover notes](docs/design/cutover-history.md).

Restrict access to state folders and backups. The library sets owner-only file
permissions on Unix. On Windows, set owner-only ACLs yourself. The library
flushes file data before replacement, and Unix directory fsync has its own
contract. Content budgets do not cap the process's resident memory.
Big-integer Diffie-Hellman is not constant time. Zeroization cannot erase every
copy the allocator or operating system makes. AES-IGE file fingerprints identify
keys; they do not verify file integrity.

Offline tests use independent vectors and a retired official-client capture.
They do not verify a live Telegram account or a `telegram-mcp` integration.
Keep the [license](LICENSE), [notices](NOTICE) and
[SDK notices](vendor/grammers/README.md) with the source.

## Donate

If this project helps you, donations are appreciated.

| Currency | Network | Address |
| --- | --- | --- |
| Bitcoin (BTC) | Bitcoin | `bc1qmth5m03pu5hujw5xw5jmywam3jj3sqwqupesdt` |
| USDT, BNB, USDC, etc. | BEP20 | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
| USDT, TRX, USDC, etc. | TRC20 | `TWBA3xFTqgZAeAYMxqo85xWnzvty3DcAhw` |
| Ethereum (ETH) | ERC20 | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
| TON | TON | `UQCN8Umo_OfOWqImZetQsrNStPcmLkMAKajFyiCOhso23NDb` |
| Litecoin (LTC) | LTC | `ltc1qntqnnrunadurnw4cshv3qgspywrueyyeyngwuy` |
| Solana (SOL) | Solana | `7B2wkczUjmkDhETwQuknBL8sUsbuV7nErxc317TmQuwR` |
| Polygon (POL) | Polygon | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
