# Rust migration and current execution evidence

The immutable Python baseline is commit
`d53c7c21574ac404e01b8c3535b0d0fcfa25a193`. It collected **1851 cases**,
including parametrization and explicitly gated live cases. `legacy-cases.json`
retains those original identities and maps each to one distinct native Rust
scenario. The individual `*-mappings.json` ledgers partition those mappings;
`rust-cases.json` records the 1851 mapped native identities.

The mapped integration scenarios are executable `tests/rust_*.rs` entry points,
with supporting fixtures and helper modules. A helper function or JSON mapping
alone is not an executed test. The restored suites retain unit/regression,
async/lifecycle, property/stateful, independent crypto/schema/export oracle,
source/Git installed-consumer, SDK boundary, and opt-in real-peer scenarios.
The subsequent core, native, and regression suites remain enabled alongside them.

## Two different checks

`migration-audit --require-complete` verifies the historical mapping: all 1851
legacy cases have distinct native identities, the owner ledgers agree, and every
mapped identity remains in the current executable inventory. This is a metadata
check. Its success does not establish that any test has run.

`matrix-evidence` verifies actual Cargo execution logs against
`current-rust-cases.json`, the separate inventory of the current source. It checks
all seven declared native CI legs, exact identities, per-suite counts, summaries,
failures, duplicate identities, and allowed ignores. A missing suite, unexpected
skip, incomplete artifact, extra test, or unreported test failure rejects the
matrix. The current inventory must be reviewed when tests are intentionally added
or changed; CI never regenerates it from a possibly incomplete run.

```sh
cargo test --locked --all-targets -- --test-threads=2
cargo test --locked --doc
cargo run --locked --bin migration-audit -- --require-complete
cargo run --locked --bin matrix-evidence -- /path/to/downloaded-seven-leg-records
```

The default evidence policy is the bundled current inventory. An explicit
`--current-manifest PATH` selects a separately reviewed current inventory.
`--manifest PATH` retains the historical 1851-case policy plus its native
supplemental cases for archived records; it is not the current CI policy.

The required **Rust CI** job downloads the seven `rust-*` log artifacts and the
Linux verifier package from the same workflow run, verifies the package hashes,
and runs both checks. This makes the gate apply to the final tested commit rather
than to an ancestor or to a green component subset.

## Explicit live and platform boundaries

Ten cases in `tests/rust_live_interop.rs` and two in
`tests/live_interop_harness.rs` are live Telegram entry points and are explicitly
ignored in normal CI. Their credential parsers, offline protocol vectors, and
local transport fixtures execute normally. The ignored cases are not reported as
passed or used as evidence of a new real-account run. Use the
[explicit native live procedure](../docs/rust-live-tests.md) or the
[current workflow harness procedure](../docs/interop-testing.md) with controlled
accounts when those checks are required.

Windows also collects and explicitly ignores three Unix filesystem contracts:
owner-only Unix permissions, symlink privileges, and directory fsync behavior.
These are different from tests compiled only under `cfg(unix)`. The current
manifest lists the latter under `platform_excluded`; it cannot relabel a collected
platform ignore as an absent test. All other collected tests must execute.

Independent reference encoders emitted the historical expected vectors before
removal of the old implementation. Project corrections and upstream updates use
explicit reviewed overlays rather than silently replacing those baseline
expectations. The burned official-client capture is historical evidence.
