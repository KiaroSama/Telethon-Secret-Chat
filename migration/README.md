# Rust migration evidence

This directory owns the data in `legacy-cases.json`. It is a migration ledger,
not an executable test registry. The baseline is main commit
`d53c7c21574ac404e01b8c3535b0d0fcfa25a193`.

The existing suite contains 932 test definitions and collects **1851 cases**,
including parametrized cases and explicitly gated account tests. Preserve the
case count and the behavior of each scenario in the final Rust cutover. Passing
new Rust tests does not prove this equivalence. Do not add duplicated tests,
placeholder passes or ignored regressions to reach a target number.

`cases` contains the original pytest node IDs. `mappings` maps an original case
ID to the fully qualified Rust test ID only after its behavior has been compared
and its actual execution verified. Values must be unique. The ledger is initially
empty rather than claiming that broadly related tests are exact replacements.

Run `cargo run --locked --bin migration-audit` for a report. Add
`-- --require-complete` for the blocking completion gate. Until every case is
mapped, the Rust migration is incomplete and must not be merged as a replacement.

The Python implementation and tests remain the reference until this gate and
integrated feature acceptance pass. `tests/fixtures/rust-schema-oracle.json`
contains synthetic bytes emitted by the unchanged baseline serializer for all 90
constructors; its provenance is recorded in the file. The official-client frame
fixture is the existing closed throwaway-chat capture, not a new live test.

`rust-cases.json` records the currently executed Rust component case identities.
The matrix gate compares every leg against this exact set and allows only four
explicitly ignored Unix filesystem tests on Windows. This separate manifest is
not evidence that those component tests replace the legacy cases.
