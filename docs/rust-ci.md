# Native Rust verification

The migration preserves 1,851 individually collected scenarios. The native
collection and the migration ledger must agree exactly; adding a passing test
does not substitute for an unmapped scenario. Property tests still generate
inputs, refusal tests exercise actual operations, and filesystem tests perform
real I/O. Independent protocol, Desktop export and image fixtures remain data.

## Build and test matrix

`.github/workflows/rust.yml` builds the library and executables on every run,
then executes all targets on these nine native legs:

| Hosted runner | Architecture | Rust | Profile |
| --- | --- | --- | --- |
| ubuntu-24.04 | x86_64 | 1.95.0 | Debug, including debug assertions |
| ubuntu-24.04 | x86_64 | 1.100.0 | Debug, with native library coverage |
| ubuntu-24.04 | x86_64 | 1.100.0 | Release |
| ubuntu-24.04 | x86_64 | 1.100.0 | Debug, forced portable AES backend |
| ubuntu-24.04-arm | aarch64 | 1.100.0 | Release |
| windows-2025 | x86_64 | 1.100.0 | Release |
| windows-11-arm | aarch64 | 1.100.0 | Release |
| macos-15-intel | x86_64 | 1.100.0 | Release |
| macos-15 | aarch64 | 1.100.0 | Release |

Every job checks the actual runner architecture and `rustc` host triple before
building. ARM64 jobs execute the full test suite natively. `RUSTUP_TOOLCHAIN`
also pins nested Cargo and rustc calls to the selected leg. Both Windows jobs
compile the committed source from a directory containing spaces and use native
PowerShell for build and test commands, retaining Cargo's actual exit status.
Windows ARM setup also avoids emulated Git Bash, which can exit without running
commands. The portable job sets `aes_backend="soft"` and checks the selected
backend against an independent AES known answer. Every leg fetches the complete
locked dependency graph before tests that deliberately run Cargo offline.

`matrix-evidence` reads each Cargo suite and its test-result summary, strips
terminal color escapes and requires exactly the same manifest on all nine legs.
Missing, additional, duplicate, failed, filtered, measured or unexpectedly
ignored cases fail the gate. A job that only prints successful test lines cannot
pass without matching suite summaries. The separate migration audit requires
all 1,851 legacy identities to have unique, collected native targets.

Ten compiled account interoperability cases are ignored during ordinary CI.
They require explicitly configured, already authorized test accounts; collecting
them does not read credentials or contact Telegram. Windows additionally ignores
exactly three Unix filesystem scenarios: owner-only store permissions, symlink
refusal and directory-sync failure handling. The manifest names each exception;
there is no general permission to ignore tests. Those three platform exceptions
apply to both Windows architectures and to no Linux or macOS leg.

The six release legs upload `native-{target-triple}` artifacts. Each contains
four native developer tools (`generate-schema`, `matrix-evidence`,
`migration-audit` and `prepare-sdk`), inherited licenses, target/toolchain/commit
metadata, and SHA-256 checksums. Unix tools are in a tar archive that preserves
executable permissions; Windows tools use ZIP. These are CI artifacts; no
repository release is published. Rust library consumers still use the complete
Git/source graph described below.

## Coverage and source checks

The Ubuntu 1.100.0 debug leg uses pinned `cargo-llvm-cov` 0.9.1 and LLVM tools,
with a **90% line-coverage gate**. Its package-library scope includes the native
protocol, storage, manager, SDK adapters and exporter. It excludes integration
tests, the explicitly named `cfg(test)` storage/ingress helper modules, generated
output, executable tools, third-party dependency sources, vendored SDK source and
the separately licensed adapted JPEG decoder. Owned JPEG/ICC/scaling adapters remain in scope. The
workflow publishes `coverage.lcov` as `native-library-coverage`.

The original Python configuration combined line and branch coverage. Stable
Rust's LLVM instrumentation exposes lines and regions; its branch instrumentation
is currently unstable. The native gate therefore measures lines explicitly,
and `coverage-summary.json` also reports actual region coverage. These are
different metrics, not an assertion of numerically identical branch coverage.
No bootstrap compiler switch or invented branch percentage is used.

The SDK fixture executable is included explicitly in coverage collection. Its
remapped library paths count toward the same package scope, including actual
transport, transfer and export RPC paths. Generated Cargo build output is
excluded for both the default and custom target directories.

The native `matrix-evidence --coverage coverage-summary.json` command enforces
90% using exact integer line counts. It checks LLVM's schema, file ownership,
duplicate records, numerical consistency and the sum of per-file totals. This
checks the same JSON that is published: pinned cargo-llvm-cov 0.9.1's
[internal threshold query omits extra object flags](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.1/src/report.rs#L864),
although its report includes them. Test execution, export and the native gate
must each succeed. Read the uploaded JSON for the current commit's measured
line, region and function percentages; line coverage is the gate.

The same leg checks Rust formatting and runs Clippy with warnings denied.
Security tests parse Rust syntax trees, so comments cannot accidentally satisfy
or trip random-source audits. Producer checks follow actual returned secrets
to OS randomness or the pinned upstream secure implementation. Error-contract
tests exercise refusal paths and assert safe, useful diagnostics.

CodeQL uses the official Rust `security-extended` query suite. Dependabot tracks
both Cargo workspaces and GitHub Actions. These jobs supplement the test matrix;
their success is checked for the current pull-request commit.

## Source and installed Git consumers

The package is distributed from Git or a complete source archive and has
`publish = false`. Its three bundled Grammers crates and pinned native image
dependency must remain in the source graph. A normalized `.crate` archive is
not presented as a registry-compatible release.

Four packaging scenarios create an isolated, allowlisted Git fixture and build
a real external installed consumer. They verify one canonical SDK graph, the
committed lockfile, a path containing spaces, license/notice propagation,
metadata-derived versioning and an actual HTML/JSON export with all 44 asset
hashes. The consumer runs from outside the checkout. Fixture Git identity is
local to each command; tests never modify the repository's history or global
Git configuration.

`package-validation.yml` checks the committed source inputs and uploads a Git
archive with its commit, file list and SHA-256 checksums. It rejects executable
legacy-language files and private working data in tracked distribution inputs.
Its `source-distribution-rust` artifact is separate from the nine matrix records.

## Native history scanner

`gitleaks.yml` builds only `tools/secret-scan`, an independent Rust workspace
with a minimal pinned lockfile and no build script. Source is copied outside
the checkout before building; the root package and repository-local Cargo
configuration are not executed by the scanner job. Diagnostic tracing is
refused before checkout.

The controller verifies Gitleaks 8.30.1 by its official download SHA-256 and
reported version. It scans full fetched history, or validated pull-request
revisions, with built-in rules and no repository allowlist. A native Git
observer verifies the actual child status, including failures that a scanner
could otherwise hide. Process groups, private output files, deadlines, report
validation and status agreement fail closed. Public output contains fixed
diagnostics and escaped finding locations; matched contents, secrets and raw
reports are never uploaded.

## Official references

- [Cargo installation and Git/path sources](https://doc.rust-lang.org/cargo/commands/cargo-install.html)
- [Tokio Notify ordering](https://docs.rs/tokio/1.54.1/tokio/sync/struct.Notify.html)
- [cargo-llvm-cov 0.9.1](https://github.com/taiki-e/cargo-llvm-cov/tree/v0.9.1)
- [Gitleaks 8.30.1](https://github.com/gitleaks/gitleaks/releases/tag/v8.30.1)
- [Official hosted runner architectures](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [Windows ARM Git Bash execution issue](https://github.com/actions/runner-images/issues/14052)

These commands and thresholds describe the checked-in policy. Hosted results
must be read against the latest pull-request SHA, including every bot report.
