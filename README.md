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
| Protocol core, TL codegen & PFS rekey | [src/protocol/](src/protocol/mod.rs), [build.rs](build.rs), [src/schema_generator.rs](src/schema_generator.rs), [src/wire.rs](src/wire.rs), [src/framing.rs](src/framing.rs), [src/rekey.rs](src/rekey.rs) | Generated Rust TL `types`/`enums`/`deserialize`/`wire` from the bundled schema, secret-chat layer support through 144, MTProto 2.0 encryption, `msg_key` KDF, sequence counters, gap queues, automatic and explicit PFS rekeying, previous-key cutover, duplicate wrapper dedup, versioned typed snapshots, and 12x12 key visualization ([src/visualization.rs](src/visualization.rs)). |
| Full managers & runtime modes | [src/manager/](src/manager/mod.rs), [src/storage.rs](src/storage.rs), [src/storage_backend/](src/storage_backend/mod.rs), [src/binding_migration.rs](src/binding_migration.rs), [src/runtime_modes/](src/runtime_modes/mod.rs) | `manager::SecretChatManager<S, T>` and `manager::StrongSecretChatManager` (rich `send_message`/`send_message_with_options`, encrypted `send_file`/`forward_file`/`receive_file`/`save_file`, establishment, actions, custody/pause, autosave, typed `Event`/`MessageReceived` callbacks, and `grammers_mtsender` `DurableIngressHook` wiring), plus `CoreSecretChatManager`, `PersistentSecretChatManager`, `BoundedSecretChatManager`, `TransientSecretChatManager`, and explicit offline `v1`-to-`v2` migration (including historical encrypted and JSON stores). |
| Custody & durable ingress | [src/custody/](src/custody/mod.rs), [src/ingress/](src/ingress/mod.rs), [src/binding.rs](src/binding.rs), [src/storage_file.rs](src/storage_file.rs) | Domain `v2` `AuthorizationBinding` (`Client::authorization_key_tag()`), `DurableIngress` pre-ack callback wiring, `EncryptedIngressStorage`, `CustodyLimits`/`StoreCustodyLimits`, `CustodyStorage`, `RelayDelivery`, atomic AES-256-GCM replacement, empty-lockfile integrity, strict duplicate-key serde rejection, stale-revision refusal, and explicit non-consuming polling + acknowledgement. |
| Encrypted media & orchestration | [src/media/](src/media/mod.rs), [src/orchestration.rs](src/orchestration.rs) | Metadata for 8 media kinds (`photo`, `video`, `document`, `audio`, `voice_note`, `video_note`, `sticker`, `gif`), streaming AES-256-IGE (`EncryptingReader`, `FileSource`, `DocumentMediaInput`, `MediaReference`), Ogg Opus header detection, forwarding descriptors, atomic no-clobber file saving, plus caller-owned `MediaOrchestrator`, `AutoSaveSubscription`, `BoundedCustodySettlement`, and bounded async `BoundedLifecycleSubscription`. |
| Auto-save & Desktop archive export | [src/autosave/](src/autosave/mod.rs), [src/export/](src/export/mod.rs), [src/export_archive.rs](src/export_archive.rs), [src/orchestration.rs](src/orchestration.rs) | Independently encrypted saved history (`AutoSaveStore`, `AutoSave`), full Telegram Desktop-compatible HTML/JSON rich exporter with embedded assets and packed-buffer ICC/JPEG thumbnail scaling, opt-in `DecodedImageBudget`, and staged atomic archive publication. |
| Maintenance utilities | [src/bin/check-test-matrix.rs](src/bin/check-test-matrix.rs), [src/bin/generate-schema.rs](src/bin/generate-schema.rs), [src/bin/matrix-evidence.rs](src/bin/matrix-evidence.rs), [src/bin/migration-audit.rs](src/bin/migration-audit.rs), [src/bin/prepare-sdk.rs](src/bin/prepare-sdk.rs) | CI matrix verification, secret-chat TL schema generation, migration ledger auditing, matrix evidence validation, and vendored SDK preparation. |

## Build and use

The repository uses Rust edition 2024 (MSRV 1.95, pinned to `1.100.0` in [rust-toolchain.toml](rust-toolchain.toml)) and vendors `grammers-client`, `grammers-mtsender`, and `grammers-session` under [vendor/grammers/](vendor/grammers/README.md) pinned to `11e13a989a754f9cf01a798480a2e20ce9e5b1fd` (Layer 234).

Add the crate to a Rust application by Git tag or commit:

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Grammers-Secret-Chat.git", tag = "v0.5.0" }
```

Read the [integration and upgrade boundaries](docs/integration-boundaries.md), including the `v0.5.0` authorization binding `v2` change and explicit offline migration helpers, before opening an existing `v1` store.

### Prebuilt signed binaries (no local build needed)

The protected release workflow packages all five maintenance utilities (`check-test-matrix`, `generate-schema`, `matrix-evidence`, `migration-audit`, `prepare-sdk`) for four 64-bit targets (`windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64`). Inspect the selected [GitHub Release](https://github.com/KiaroSama/Grammers-Secret-Chat/releases) for its actual archives, `.sig` files, `SHA256SUMS`, `SHA256SUMS.sig`, and publisher verification material. Verify SHA-256 and RSA-SHA256 signatures against [docs/signing/publisher.json](docs/signing/publisher.json) before installing the utilities inside `bin/`. These utilities are separate from linking the Rust library into an application.

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
- See [docs/architecture.md](docs/architecture.md), [docs/bounded-mode.md](docs/bounded-mode.md), [docs/transient-mode.md](docs/transient-mode.md), [docs/strong-ingress.md](docs/strong-ingress.md), [docs/protocol-reference.md](docs/protocol-reference.md), [docs/benchmarks/checkpoint-storage.md](docs/benchmarks/checkpoint-storage.md), and [docs/interop-testing.md](docs/interop-testing.md) for full contracts.

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
