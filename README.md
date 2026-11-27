<div align="center">

# Kiaro Secret Chat (Rust)

**Telegram MTProto 2.0 end-to-end encrypted secret chats for a caller-owned grammers client in pure Rust.**

[![GitHub Actions CI across 64-bit Linux and Windows](https://img.shields.io/github/actions/workflow/status/KiaroSama/Telethon-Secret-Chat/rust.yml?branch=main&style=flat-square&color=16a34a&label=CI)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/rust.yml)
[![GNU General Public License v3.0 or later](https://img.shields.io/github/license/KiaroSama/Telethon-Secret-Chat?style=flat-square&color=2563eb)](LICENSE)
[![Latest GitHub release](https://img.shields.io/github/v/release/KiaroSama/Telethon-Secret-Chat?style=flat-square&color=ea580c)](https://github.com/KiaroSama/Telethon-Secret-Chat/releases)
[![68 native Rust tests and 92/92 rust-anything verified units](https://img.shields.io/badge/tests-68%20native%20%7C%2092%2F92%20units-0d9488?style=flat-square)](#verification-and-migration-evidence)
[![Rust 1.100.0 measured locally, edition 2024, MSRV 1.95](https://img.shields.io/badge/Rust-1.100.0-8b5cf6?style=flat-square)](rust-toolchain.toml)
[![Runs on 64-bit Linux and Windows](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-0891b2?style=flat-square)](.github/workflows/rust.yml)
[![Install from Git or signed release archives](https://img.shields.io/badge/install-Git%20%7C%20release-4f46e5?style=flat-square)](#install-and-usage)
[![Protocol reference, architecture, and ADRs](https://img.shields.io/badge/docs-protocol%20%7C%20ADRs-0284c7?style=flat-square)](docs/protocol-reference.md)
[![Live GitHub repository size](https://img.shields.io/github/repo-size/KiaroSama/Telethon-Secret-Chat?style=flat-square&color=ca8a04)](https://github.com/KiaroSama/Telethon-Secret-Chat)
[![Built with grammers 0.12.0 at 11e13a98 Layer 234](https://img.shields.io/badge/built%20with-grammers%200.12.0-15803d?style=flat-square)](vendor/grammers/README.md)
[![Secret chats use MTProto 2.0 layers 73 through 144](https://img.shields.io/badge/protocol-MTProto%202.0%20L73--144-b45309?style=flat-square)](docs/protocol-reference.md)
[![Native builds target x64 and arm64 only](https://img.shields.io/badge/arch-x64%20%7C%20arm64-334155?style=flat-square)](.github/workflows/rust.yml)
[![Crate forbids unsafe Rust code](https://img.shields.io/badge/safety-unsafe%20forbidden-7c3aed?style=flat-square)](Cargo.toml)
[![CodeQL static analysis and Gitleaks secret scanning](https://img.shields.io/badge/security-CodeQL%20%7C%20Gitleaks-c2410c?style=flat-square)](.github/workflows/codeql.yml)
[![Support donations](https://img.shields.io/badge/Support-donations-d04a9a?style=flat-square)](#donate)

</div>

`kiaro-secret-chat` adds Telegram MTProto 2.0 secret chats to a caller-owned [grammers](https://codeberg.org/Lonami/grammers) client in pure Rust. The crate version is `0.3.0` (`kiaro_secret_chat` in Rust), with zero Python runtime and zero PyO3 dependencies.

## Features

The library implements the MTProto 2.0 secret-chat protocol (layers 73 through 144): 2048-bit RFC 3526 Group 14 Diffie-Hellman handshakes using OS entropy (`getrandom`), SHA-256 `msg_key` and KDF (`x=0` and `x=8`), AES-256-IGE framing, SHA-1 key fingerprints, fail-closed sequence ordering, key rotation, and zeroized key slots. Errors carry safe diagnostic fields without exposing keys, nonces, or plaintext.

Storage and ingress are caller-controlled. Durable ingress (`ingress::EncryptedIngressStore` and `DurableIngressController`) and custody (`custody::CustodyStore`) persist encrypted AES-256-GCM state through atomic file replacement, require an explicit absolute path (no working-directory fallback), and preserve caller-owned authorization identity.

The export subsystem preserves Telegram Desktop HTML and JSON formatting, byte-exact Qt 5.15.19 ICC sRGB color transforms, JPEG thumbnail scaling, QUrl encoding, MIME glob resolution, and all 46 static viewer assets under `assets/tdexport/` without any JavaScript build toolchain.

## Install and usage

Add `kiaro-secret-chat` to your `Cargo.toml` from this repository:

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Telethon-Secret-Chat.git", branch = "main" }
```

Build the library and both maintenance binaries locally (Rust 1.95+ MSRV, tested with Rust 1.100.0):

```powershell
cargo test --locked --all-targets
```

```powershell
cargo build --locked --release --bins
```

The repository ships two standalone maintenance executables:

- `check-test-matrix`: validates test-matrix declarations against `LIVE_SKIPS` and reports missing or undeclared entries.
- `generate-schema`: parses `schema/end-to-end.tl` and emits the deterministic schema manifest (`--schema`, `--out`, `--check`).

Signed release archives for 64-bit Linux (`x86_64`, `aarch64`) and 64-bit Windows (`x86_64`, `aarch64`) include both binaries, `README.md`, `LICENSE`, `NOTICE`, `build-info.json`, and `SHA256SUMS`, signed inside the protected `numera-signing` GitHub Actions environment. Verify signatures against the public key in [docs/signing/publisher.json](docs/signing/publisher.json).

## Verification and migration evidence

This Rust codebase was rewritten from frozen Python baseline commit `5b68ef95bacbcc6d520f960899fca58ad447e7ea` through `rust-anything 5.0.1`. Every one of the 92 non-test baseline source files (334 classes, 1,568 definitions, 48 public exports, and 933 offline test declarations) was reconciled in [migration/019-rust-anything-receipt.json](migration/019-rust-anything-receipt.json):

- 38 crates passed raw `rust-anything gate` 15/15 (exit 0).
- 54 crates passed 100% differential replay, fuzzing, benchmarking, and shadow generation through deterministic adapters.
- All runtime modules were lifted to pure Rust under `src/` with zero PyO3 dependencies and `#![forbid(unsafe_code)]`.
- The vendored transport under [vendor/grammers/](vendor/grammers/README.md) pins upstream `Lonami/grammers` commit `11e13a989a754f9cf01a798480a2e20ce9e5b1fd` (Telegram API Layer 234, including TCP `nodelay`) while preserving local credentialed SOCKS5 and durable-ingress patches across 88 verified SHA-256 hashes.
- 68 native Rust unit and integration tests verify cryptographic vectors, the retired official-client capture, all 16 byte-exact Qt 5.15.19 ICC goldens, all 46 static export assets, all 58 ICC fixtures, durable ingress, custody controls, and both CLI tools.

## Security and limits

The library forbids `unsafe` Rust code in the crate root, redacts secrets from error messages, and flushes encrypted store files before atomic replacement. On Unix systems, store files enforce restricted permissions; on Windows, configure owner-only ACLs on the chosen storage directory. Big-integer Diffie-Hellman arithmetic is not constant time, and zeroization cannot erase copies retained by the OS or allocator. Keep [LICENSE](LICENSE), [NOTICE](NOTICE), and [vendor/grammers/README.md](vendor/grammers/README.md) with redistributed source or binaries.

## Author

Maintained by [KiaroSama](https://github.com/KiaroSama) (`Kiaro.Sama.Dev@gmail.com`).

## License

`kiaro-secret-chat` is copyright KiaroSama and licensed under the GNU General Public License v3.0 or later ([GPL-3.0-or-later](LICENSE)).

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
