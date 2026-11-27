<div align="center">

# Kiaro Secret Chat (Rust)

**Rust protocol, runtime, and encrypted storage components for Telegram MTProto 2.0 secret chats.**

[![Rust CI](https://img.shields.io/github/actions/workflow/status/KiaroSama/Telethon-Secret-Chat/rust.yml?branch=main&style=flat-square&label=CI)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/rust.yml)
[![Latest repository tag](https://img.shields.io/github/v/tag/KiaroSama/Telethon-Secret-Chat?sort=semver&style=flat-square&label=tag)](https://github.com/KiaroSama/Telethon-Secret-Chat/tags)
[![GPL-3.0-or-later](https://img.shields.io/github/license/KiaroSama/Telethon-Secret-Chat?style=flat-square)](LICENSE)
[![Rust toolchain](https://img.shields.io/badge/Rust-toolchain-8b5cf6?style=flat-square)](rust-toolchain.toml)
[![Configured native targets](https://img.shields.io/badge/targets-Linux%20%7C%20Windows%20%C2%B7%20x64%20%7C%20arm64-0891b2?style=flat-square)](.github/workflows/rust.yml)
[![Protocol and architecture](https://img.shields.io/badge/docs-protocol%20%7C%20architecture-0284c7?style=flat-square)](docs/architecture.md)

</div>

`kiaro-secret-chat` exposes Rust APIs under `kiaro_secret_chat`. It includes authenticated secret-chat framing, live sequence and rekey state, an in-memory manager, and separate encrypted payload stores. The caller supplies the transport adapter and owns authorization, network connection, and application persistence.

**The Rust crate does not provide complete behavioral parity with the retired Python package.** It has no complete persisted manager, initial request/accept network adapter, protected transient manager, native media transfer lifecycle, or saved-chat archive API. The historical migration inventory is evidence about adapter work, not a substitute for those implementations. [Current architecture and capability boundaries](docs/architecture.md) describe the supported surface.

## What is available

| Area | Current Rust scope |
| --- | --- |
| Cryptography and framing | OS entropy; validated 2048-bit DH arithmetic; SHA-256 message keys and KDF; AES-256-IGE; key fingerprints; strict authenticated frame and TL validation. |
| Live protocol state | Role/parity checks, bounded gap buffering, ordered batch delivery, replay rejection, resend state, separate peer capability and wrapper layers, and runtime rekey with active/pending/previous key handling. |
| Manager | Caller-registered established chats, serialized retained sends, identical-byte retries after uncertain transmission, receive batches, protocol service handling, events, close, and shutdown in the current process. |
| Custody | Bounded payload containers and AES-256-GCM snapshots with explicit paths, authorization checks, atomic replacement, and failure-preserving retention. They do not serialize a complete live chat. |
| Ingress | Encrypted opaque-record capture, deduplication, sorted pending snapshots, and explicit durable acknowledgement. No verified transport baseline or contiguous protocol-applied cursor is installed automatically. |
| Export helpers | HTML/JSON escaping and formatting, URL validation, MIME lookup, ICC conversion, JPEG metadata/thumbnail helpers, and retained Desktop viewer assets. These are components rather than a complete saved-chat exporter. |
| Media and archives | Media vocabulary constants exist. The Rust manager does not implement the Python upload/download/forward, auto-save, saved-history, or archive export APIs. |

See [the manager](src/manager.rs), [protocol core](src/protocol.rs), [rekey implementation](src/ported/rekey.rs), [custody](src/custody.rs), [ingress](src/ingress.rs), and [export helpers](src/export.rs) for the actual compiled APIs. The crate implements MTProto 2.0; it does not fall back to MTProto 1.0. Outgoing secret-message layer encoding is capped at layer 144. The outer Telegram API layer is a separate transport concern.

## Integration boundary

Supply an implementation of `manager::SecretTransport` around your existing authorized client. The manager neither creates a login nor supplies an automatic `grammers_client::Client` adapter. Implement `send_encrypted_service` with `messages.sendEncryptedService`; its default implementation refuses. Ordinary encrypted messages and encrypted service messages must use their appropriate Telegram RPCs.

`register_established_chat` installs a key and identifiers that the caller has already established. DH validation, public-value/shared-key helpers, and rekey support do not perform the initial `getDhConfig`, `requestEncryption`, or `acceptEncryption` exchange for the caller. Call `configure_chat_dh` with the chat's original validated DH parameters before the automatic rekey threshold; the manager also exposes `start_rekey` and `send_service_action`.

Feed received ciphertext through the batch receive API so a newly filled gap can release every accepted message. Drain and process manager events according to your application policy. The event queue is caller-drained; registered IDs, including closed-chat tombstones, remain until the manager is dropped. Bound the number of registrations and the lifetime of each manager instance. Outbox and gap bounds are not a process-wide RSS or task budget.

An inbound message can be accepted and its events queued before transmission of an automatic service reply fails. In that case `receive_frames` can return `SendPending`; inspect `drain_events` for the accepted inbound outcome and use `retry_pending` for the retained reply. Do not interpret every receive error as proof that inbound state was unchanged.

A successful send RPC, peer sequence acknowledgement, ingress-store acknowledgement, application settlement, and user read receipt are different facts. There is no automatic read-receipt or external relay settlement policy.

### Persistence and operational modes

- [Bounded custody](docs/bounded-mode.md) explains count/byte refusal, preserved payloads, local pause flags, and the absence of a bounded protocol manager.
- [Transient scope](docs/transient-mode.md) explains the volatile mode label and why it is not the retired protected transient manager.
- [Ingress storage](docs/strong-ingress.md) explains recovery of captured records, explicit acknowledgement, and the missing transport/protocol checkpoint integration.

Restarting a `CustodyStore` or `DurableIngressController` reconstructs its own records. It does not restore the manager's keys, sequence counters, rekey exchange, outbox, gaps, or events. Do not attach a restarted manager to an old secret chat using counters reset to zero.

## Build and use

The package metadata and minimum Rust version are in [Cargo.toml](Cargo.toml); the development toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml).

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Telethon-Secret-Chat.git", branch = "main" }
```

For reproducible deployments, replace the moving branch with a reviewed commit revision and retain the resolved lockfile. The crate is a library, not a standalone secret-chat client.

From a checkout, run these commands in PowerShell 7 or your platform's shell:

```powershell
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --locked --release --bins
```

The library has no Python runtime or PyO3 dependency. Its JPEG path depends on `mozjpeg`/`mozjpeg-sys` and a native C codec build, so the complete dependency tree is **not pure Rust**. A supported native build environment is required; the CI workflows show the configured Linux and Windows targets.

The two maintenance binaries are:

- `check-test-matrix`: checks the declared test-matrix evidence and allowed skips.
- `generate-schema`: parses the tracked end-to-end TL schema into a deterministic constructor manifest; supports generation and `--check`.

Release workflows describe packaging and signing of these utilities. An archive containing them is not a packaged manager daemon or a replacement for linking the library. Check the actual [release assets](https://github.com/KiaroSama/Telethon-Secret-Chat/releases), their workflow results, signatures, and [publisher material](docs/signing/publisher.json) for the revision you use.

## Verification and migration evidence

Run the current tests and inspect CI for the exact commit being reviewed. Test totals are discovered from source and execution; no badge in this README asserts a fixed count or complete protocol interoperability.

The [migration receipt](migration/019-rust-anything-receipt.json) records the original migration's reported results against frozen Python baseline `5b68ef95bacbcc6d520f960899fca58ad447e7ea`. Its file/symbol totals and adapter gates are retained as historical evidence. Some entries explicitly describe asynchronous shells or archive integration still pending. They do not establish that every Python manager, event, storage backend, or media API is implemented in the current crate.

[public_api.rs](src/public_api.rs) is an audited metadata inventory of historical names. Its strings do not create Rust aliases, exports, or working compatibility implementations. Entries distinguish actual native items, partial capabilities, and unavailable features. [The public contract tests](tests/public_contract.rs) check that distinction and compile concrete native types.

Current tests include cryptographic vectors, a retired official-client fixture, protocol/manager simulations, storage fault cases, and export goldens. Offline fixtures and simulated transports do not prove a live official-client or consumer deployment. The vendored grammers revision and local transport patches are documented in [vendor/grammers/README.md](vendor/grammers/README.md); these patches alone do not establish the missing manager/ingress integration.

## Security and limits

This crate forbids `unsafe` Rust in its own source. That restriction does not apply transitively to dependencies or the native codec. Public payload-container diagnostics summarize sizes and structural state; applications must still keep keys, payloads, authorization material, and arbitrary transport errors out of their own logs.

Encrypted stores use an explicit absolute path, owner-only Unix file permissions, exclusively created staging files, file flushing, atomic replacement, and Unix parent-directory flushing. A persistent sidecar lock coordinates cooperating writers; ingress also refuses stale snapshot replacement. Configure an owner-only storage directory, including Windows ACLs, and keep encryption keys separately. An uncertain post-replacement failure requires reopening and reconciliation. Encryption does not prevent rollback to an older authentic snapshot without a separate trusted anchor.

The manager's retained sends and live protocol state are volatile. Cancellation cannot retract an RPC already issued to a transport, and shutdown is not proof that every caller-owned network future stopped. Big-integer DH is not constant time; zeroization cannot erase all allocator, operating-system, or caller-held copies. No physical-erasure or universal power-loss guarantee is made.

Keep [LICENSE](LICENSE), [NOTICE](NOTICE), and the applicable vendored notices with redistributed code. The [protocol reference](docs/protocol-reference.md) and [ADRs](docs/adr/) preserve protocol reasoning; historical Python design notes do not override the current Rust capability boundaries.

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
