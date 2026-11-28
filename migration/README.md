# Rust migration evidence

The immutable baseline is main commit `d53c7c21574ac404e01b8c3535b0d0fcfa25a193`.
It collected **1851 actual cases**, including parametrization and explicitly gated
live cases. `legacy-cases.json` retains those original IDs as data and maps each
to one distinct native Rust scenario. Individual `*-mappings.json` ledgers own the
reviewed mappings; `rust-cases.json` records the 1,851 mapped native identities.

The executable suite retains scenario types: unit/regression, async/lifecycle,
property/stateful, independent crypto/schema/export oracle, source/Git installed
consumer, SDK boundary and opt-in real-peer tests. Extra foundation regressions
are folded into related scenarios, preserving assertions without inflating the
collected count. No placeholder success or newly ignored regression fills a gap.

```sh
cargo test --locked --all-targets
cargo run --locked --bin migration-audit -- --require-complete
cargo run --locked --bin matrix-evidence -- /path/to/downloaded-seven-leg-records
```

Completeness requires all 1,851 legacy cases to map to distinct native identities.
The matrix verifier checks those identities plus four native AES-IGE tests, for
1,855 total. It checks actual rows, per-suite summaries, skip policy, and agreement
across all seven legs. A green component subset or ancestor commit does not
qualify the current source.

Ten real-peer cases in `rust_live_interop` are explicitly ignored by default.
Windows additionally ignores three Unix filesystem cases: owner-only permissions,
symlink privilege and directory-fsync behavior. All other cases must run. Optional
fixture experiments in verifier tests do not authorize extra real matrix ignores.

Independent reference encoders emitted static expected vectors before removing
the old implementation; tests do not use the new serializer to invent answers.
The burned official-client fixture is historical, not a new real account run.
Live Rust execution requires the [explicit opt-in procedure](../docs/rust-live-tests.md).
