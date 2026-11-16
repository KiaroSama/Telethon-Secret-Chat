# Rust migration and SDK compatibility

`kiaro-secret-chat` is a Rust 2024 source/Git crate, version `0.3.0-alpha.1`, MSRV
1.95. The development toolchain and dependency graph are pinned. PR #33 remains
unmerged. The completeness gate and final-head matrix must agree on all 1851
collected cases; passing a component subset does not establish completion.

## One SDK graph

Official source: [Lonami/grammers](https://codeberg.org/Lonami/grammers) revision
`e61cf8585e1e9969ab81eda9fb4725a1500e2d86`, SDK 0.12.0, parser 2.0.0, API layer 233.
Three Rust crates are bundled under `vendor/grammers`:

- `grammers-client`: unchanged upstream source; its manifest selects the coherent
  bundled sender/session graph.
- `grammers-session`: full bounded MessageBoxes checkpoint and envelope/output
  codecs including gaps, deadlines, counters and difference ownership.
- `grammers-mtsender`: pre-start attachment, one update receiver, pre-ack commit,
  caller-issued difference capture and acknowledgement fencing.

Original licenses and baseline source hashes are retained. Other SDK dependencies
use the same official Codeberg pin. Use this crate's SDK re-exports: separately
sourced SDK crates have different Rust types. Path dependencies work transitively
for Git consumers without a root-only Cargo `[patch]`. Installed SDK files are
never modified at runtime. Read [strong ingress](strong-ingress.md).

```sh
cargo run --locked --bin prepare-sdk -- --source /path/to/official/grammers --output /path/to/new/output
```

The downloaded native tools work without a project checkout. `prepare-sdk`
requires the pinned official source for verification. `generate-schema` includes
the default schema; it requires `rustfmt`, or an explicit `--formatter` executable.

This optional native preparation tool validates the official checkout's pinned
hashes before writing output. The source/Git distribution already contains the
prepared SDK crates. It needs no wheel/runtime patch step. `publish = false` is
intentional: no standalone registry package is advertised for this unpublished
graph. The packaging tier installs a temporary Git dependency and runs a consumer
outside the checkout, checking lockfile, graph, notices, version and all assets.

## Format and API boundary

Typed Rust inputs replace dynamic records. Key lengths, unsigned file sizes and
TL families are checked before effects; wire/persisted bytes remain validated.
Use validated library entry points for untrusted TL input; low-level generated
`Deserializable` is an unchecked primitive.

The manager verifies the existing client authorization and exposes detached
snapshots without shared-key overrides. `Protocol<S>` is available to callers
that explicitly own transport and commit/effect ordering.

Native format 1 is incompatible with Python stores/protected formats, Telethon
sessions and patched wheels. No automatic importer is implemented. A schema,
format or binding mismatch refuses participation without overwriting the input.
Optional native-field compatibility does not imply Python byte-format support.
See [cutover guidance](design/cutover-history.md).

## Evidence and references

The immutable baseline is `d53c7c21574ac404e01b8c3535b0d0fcfa25a193`.
`migration/legacy-cases.json` maps its 1851 identities to distinct native scenarios;
`rust-cases.json` is actual Cargo collection. Independent legacy encoders produced
static expected fixtures before old code removal. Tests do not serialize their
own expected answers. The HTML viewer's static JS/CSS remain data.

Nine CI legs build before tests, including native ARM64 and x86_64 for all three operating systems. Evidence rejects partial, failed, filtered,
missing/extra or duplicate runs and undeclared ignores. Ten live cases are ignored
by default. Three Unix filesystem cases are additionally ignored on Windows.
A portable-AES leg executes the same inventory. See [verification](../migration/README.md).

Current upstream references reviewed:

- [Telegram e2e](https://core.telegram.org/api/end-to-end) and [secret schema](https://core.telegram.org/schema/end-to-end)
- [TDLib SecretChatActor](https://github.com/tdlib/td/blob/c8828d50f5eac2fc7089ce1089d7ce9095c61b5e/td/telegram/SecretChatActor.cpp)
- [grammers pinned source](https://codeberg.org/Lonami/grammers/commit/e61cf8585e1e9969ab81eda9fb4725a1500e2d86)
- [Archived implementation](https://github.com/painor/telethon-secret-chat/tree/32e7585ba4f5dbb2c6f025047c11fa52cf55d071)
- [Rust CodeQL support](https://codeql.github.com/docs/codeql-overview/supported-languages-and-frameworks/)

No real account run is implied by offline evidence. The live tier is explicitly
opt-in. `telegram-mcp` remains a future, separately requested integration.
