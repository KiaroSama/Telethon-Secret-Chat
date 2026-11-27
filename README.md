<div align="center">

# Grammers Secret Chat

**Pure-Rust Telegram MTProto 2.0 end-to-end encrypted secret chats for a caller-owned `grammers` client.**

[![Rust CI](https://img.shields.io/github/actions/workflow/status/KiaroSama/Grammers-Secret-Chat/rust.yml?branch=main&style=flat-square&color=16a34a&label=CI)](https://github.com/KiaroSama/Grammers-Secret-Chat/actions/workflows/rust.yml)
[![GPL-3.0-or-later](https://img.shields.io/github/license/KiaroSama/Grammers-Secret-Chat?style=flat-square&color=2563eb)](LICENSE)
[![Latest repository tag](https://img.shields.io/github/v/tag/KiaroSama/Grammers-Secret-Chat?sort=semver&style=flat-square&color=ea580c&label=tag)](https://github.com/KiaroSama/Grammers-Secret-Chat/tags)
[![Native first-party, vendor, and scanner tests](https://img.shields.io/badge/tests-native%20%7C%20vendor%20%7C%20scanner-0d9488?style=flat-square)](#verification-and-testing)
[![Rust 1.100.0 toolchain, edition 2024, MSRV 1.95](https://img.shields.io/badge/Rust-1.100.0-8b5cf6?style=flat-square)](rust-toolchain.toml)
[![Supported platforms: 64-bit Linux and Windows](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-0891b2?style=flat-square)](.github/workflows/rust.yml)
[![Install from Git or signed release archives](https://img.shields.io/badge/install-Git%20%7C%20release-4f46e5?style=flat-square)](#build-and-use)
[![Protocol reference, architecture, and ADRs](https://img.shields.io/badge/docs-protocol%20%7C%20architecture-0284c7?style=flat-square)](docs/architecture.md)
[![Live GitHub repository size](https://img.shields.io/github/repo-size/KiaroSama/Grammers-Secret-Chat?style=flat-square&color=ca8a04)](https://github.com/KiaroSama/Grammers-Secret-Chat)
[![Built with pinned grammers 0.12.0 on Layer 234](https://img.shields.io/badge/built%20with-grammers%200.12.0-15803d?style=flat-square)](vendor/grammers/README.md)
[![MTProto 2.0 secret-chat protocol layers 46 to 144](https://img.shields.io/badge/MTProto%202.0-layers%2046--144-b45309?style=flat-square)](docs/protocol-reference.md)
[![No unsafe code in first-party Rust sources](https://img.shields.io/badge/unsafe-forbidden-059669?style=flat-square)](src/lib.rs)
[![Signed 4-target prebuilt archives](https://img.shields.io/badge/releases-signed%204--target-7c3aed?style=flat-square)](https://github.com/KiaroSama/Grammers-Secret-Chat/releases)
[![Standalone audited secret scanner](https://img.shields.io/badge/security-Gitleaks%20%7C%20CodeQL-dc2626?style=flat-square)](.github/workflows/gitleaks.yml)
[![Support donations](https://img.shields.io/badge/Support-donations-d04a9a)](#donate)

</div>

`Grammers Secret Chat` (`kiaro-secret-chat`) implements Telegram's MTProto 2.0 end-to-end encrypted secret chats for applications that already own an authenticated [`grammers`](https://codeberg.org/Lonami/grammers) client. It reuses that client without performing account login or starting another client, forbids `unsafe` Rust across first-party source, and carries zero Python or PyO3 dependencies. Native image processing uses the `mozjpeg` C dependency.

## Features and module layout

| Area | Modules | Capabilities |
| --- | --- | --- |
| Network transport & establishment | [src/transport/](src/transport/mod.rs), [src/establishment.rs](src/establishment.rs) | Caller-owned `GrammersTransport` over Layer 234 (`getDhConfig`, `requestEncryption`, `acceptEncryption`, `discardEncryption`, `sendEncrypted`, `sendEncryptedService`, `sendEncryptedFile`, `setEncryptedTyping`, `readEncryptedHistory`, encrypted file upload/download, `adapt_update`), full Diffie-Hellman validation, and 12x12 key visualization PNG rendering. |
| Protocol core & PFS rekey | [src/protocol/](src/protocol/mod.rs), [src/ported/rekey.rs](src/ported/rekey.rs) | MTProto 2.0 encryption, `msg_key` KDF, sequence counters, gap queues, automatic and explicit PFS rekeying, previous-key sequence cutover, duplicate wrapper dedup before inner TL decode, and validated runtime snapshots. |
| Persistent & mode managers | [src/manager/](src/manager/mod.rs), [src/storage_backend/](src/storage_backend/mod.rs), [src/runtime_modes/](src/runtime_modes/mod.rs) | `PersistentSecretChatManager` checkpoints established chats with file-backed restart continuation; `MemoryStorage` is process-local. Also includes `BoundedSecretChatManager`, RAM-only `TransientSecretChatManager` with permanent restart suspension, and `StrongSecretChatManager` with durable accepted batches and explicit consumer acknowledgement. |
| Custody & durable ingress | [src/custody.rs](src/custody.rs), [src/ingress/](src/ingress/mod.rs), [src/storage_file.rs](src/storage_file.rs) | AES-256-GCM encrypted atomic file replacement, owner-bound custody stores, cooperative empty-lockfile integrity, strict duplicate-key serde rejection, stale-revision refusal, and explicit non-consuming polling + acknowledgement. |
| Encrypted media | [src/media.rs](src/media.rs) | Metadata for 8 media kinds (`photo`, `video`, `document`, `audio`, `voice_note`, `video_note`, `sticker`, `gif`), streaming AES-256-IGE, file key/IV fingerprint checks, Ogg Opus header detection, forwarding descriptors, and atomic no-clobber file saving. Applications compose these helpers with the transport RPCs; managers do not orchestrate media transfers. |
| Auto-save & archive export | [src/autosave.rs](src/autosave.rs), [src/export.rs](src/export.rs), [src/export_archive.rs](src/export_archive.rs) | Explicit caller-driven `AutoSaveStore::record_message`, independently encrypted saved history, and Desktop-style HTML/JSON export of saved plain text and media records to a new or empty destination. Completed archives publish together; rich entities, service reconstruction, and archive append are outside this API. |
| Maintenance utilities | [src/bin/check-test-matrix.rs](src/bin/check-test-matrix.rs), [src/bin/generate-schema.rs](src/bin/generate-schema.rs) | Deterministic CI matrix verification and secret-chat TL schema manifest generation. |

## Build and use

The repository uses Rust edition 2024 (MSRV 1.95, pinned to `1.100.0` in [rust-toolchain.toml](rust-toolchain.toml)) and vendors `grammers-client`, `grammers-mtsender`, and `grammers-session` under [vendor/grammers/](vendor/grammers/README.md) pinned to `11e13a989a754f9cf01a798480a2e20ce9e5b1fd` (Layer 234).

Add the crate to a Rust application by Git revision:

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Grammers-Secret-Chat.git", tag = "v0.4.0" }
```

The immutable `v0.4.0` tag predates the unreleased audit corrections. To use reviewed corrections before their next release, pin the reviewed commit with `rev` instead of `tag`. Read the [integration and upgrade boundaries](docs/integration-boundaries.md), including the authorization binding `v2` change, before opening an existing store.

### Prebuilt signed binaries (no local build needed)

The protected release workflow packages the two maintenance utilities for four 64-bit targets (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64`). Inspect the selected [GitHub Release](https://github.com/KiaroSama/Grammers-Secret-Chat/releases) for its actual archives, `.sig` files, `SHA256SUMS`, `SHA256SUMS.sig`, and publisher verification material. Verify SHA-256 and RSA-SHA256 signatures against [docs/signing/publisher.json](docs/signing/publisher.json) before installing the utilities inside `bin/`. These utilities are separate from linking the Rust library into an application.

### Development checks

```powershell
cargo fmt --all --check
cargo clippy --locked --all-targets --no-deps -- -D warnings
cargo test --locked
```

## Runtime logging

Maintenance binaries write structured UTC logs under `<launcher-root>/logs/<tool-name>_YYYY-MM-DD_HH-mm-ss_UTC.log` with format `[YYYY-MM-DD HH:mm:ss UTC] [LEVEL] [COMPONENT] Message`. No keys, ciphertexts, payloads, or credentials are written to logs or error messages.

## Security and storage boundaries

- First-party Rust sources forbid `unsafe` code (`#![forbid(unsafe_code)]`) and zeroize secret keys and temporary crypto buffers on drop.
- All encrypted stores require an explicit absolute path (never defaulting to CWD), exclusive staging files, flushing, atomic replacement without an unlink gap, and cooperative empty-lockfile guards. Callers own directory ACLs, key custody, and anti-rollback anchors.
- See [docs/architecture.md](docs/architecture.md), [docs/bounded-mode.md](docs/bounded-mode.md), [docs/transient-mode.md](docs/transient-mode.md), [docs/strong-ingress.md](docs/strong-ingress.md), and [docs/protocol-reference.md](docs/protocol-reference.md) for full contracts.

## Verification and testing

The primary Rust workflow runs on every push, pull request, and merge group: 64-bit Linux (x64, arm64, MSRV 1.95, `portable-aes`), Windows (x64, arm64), vendored `grammers` tests, standalone scanner tests, and source distribution validation. Gitleaks checks pushes and pull requests. CodeQL Rust analysis checks `main` pushes, pull requests targeting `main`, and its scheduled/manual runs. Check the exact head commit's results before merging. Release tags additionally run protected `numera-signing` packaging across all four targets; ordinary test runs do not establish live Telegram interoperability or release signing.

## Author

Created and maintained by [KiaroSama](https://github.com/KiaroSama) (`Kiaro.Sama.Dev@gmail.com`).

## License

Copyright (C) 2026 KiaroSama. Licensed under the GNU General Public License v3.0 or later; see [LICENSE](LICENSE) and [NOTICE](NOTICE).

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
