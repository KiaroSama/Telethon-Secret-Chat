# Native source and error privacy audits

The migration preserves all 91 source-randomness and 79 error-privacy scenarios from the original suite. Each collected scenario has its own Rust test and ledger entry in `migration/security-mappings.json`.

## Randomness and source discovery

`tests/support_security/scopes.json` records the native responsibility corresponding to every one of the 88 original module audit cases. Where Rust consolidated modules, explicit type, implementation or method selectors preserve distinct responsibility scopes. Exporter splits, the SDK checkpoint supplement, storage projections and manager lifetime rules are named directly. A separate recursive aggregate audits every Rust source below `src/` and `vendor/grammers/`, the build script and all three actual generated secret-schema files. New native modules are therefore covered even before a legacy-scope mapping is expanded.

The auditor parses Rust through `syn`, including imports, renamed imports, external crates, fully qualified paths and unexpanded macro token groups. Comments and string literals containing words such as `random` are not executable RNG use. Synthetic negative controls verify that insecure imports, calls and macro bodies are detected. Deterministic userland RNG crates are refused in owned protocol paths; production generation uses the operating-system `getrandom` entry point.

The secure-producer case checks real function bodies, not unused imports. It follows crypto padding and IDs, wrapper random bytes, independent file keys and IVs, DH secret exponents, handshake/rekey creation and strong-ingress nonces/challenges. The direct `getrandom::fill` calls must mutate the buffer returned by the producer. Exchange identities retain the full eight random bytes. Safe-prime testing delegates to the pinned official Grammers implementation; no predictable test RNG is injected into production.

## Errors carry shape and conversation context

The 79 error cases exercise real DH, fingerprint, ciphertext, wrapper, sequence, resend, file and manager refusals with public burned key/plaintext/ciphertext markers. Error displays and debug output must exclude those markers and long opaque runs. Useful-error checks retain the original requirement that contextual refusals identify chat 42 and state a concrete reason.

Low-level algorithms are account-independent. Callers can map their error through `Error::with_chat(chat_id)` to obtain a sealed `ChatContext` retaining the typed refusal and a fixed diagnostic. This replaces the original Python `chat_id=` keyword at those boundaries. It does not format a wire object or accept a free-form protocol reason string.

The unified Rust error enum is audited against an explicit complete privacy classification. Ordinary media input validation may identify the caller's supplied metadata, corresponding to the original non-protocol `ValueError` behavior. Contextualizing those errors strips metadata, and a real Protected-manager filename-mismatch refusal confirms that restricted execution exposes only a fixed shape.

Rust requires a `Storage` implementation in the `Protocol<S>::open` constructor. The missing-backend case therefore checks the mandatory generic bound and argument in the Rust AST and separately drives a real failing backend. It does not introduce an unused runtime `StorageRequired` variant. Stored-state corruption still has a distinct native validation refusal. Existing generated TL definitions omit debug formatting for secret-bearing objects.

These tests use deterministic fake transports and memory or temporary-file data only. They create no Telegram authorization, session or network connection.
