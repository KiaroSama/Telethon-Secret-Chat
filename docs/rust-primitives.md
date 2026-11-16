# Native protocol primitives and schema

The native primitive ledger preserves 344 collected legacy cases. Nine manager/media scenarios from `test_file_and_parser_durability.py` are separately owned by the manager ledger; they are not counted twice. Each entry in `migration/primitive-mappings.json` identifies one actual Rust test, including the filesystem fault case implemented inside the library to reach a private synchronization seam.

| Original responsibility | Cases in this ledger |
| --- | ---: |
| File keys, streaming, transport and atomic saves | 46 |
| MTProto padding | 37 |
| DH parameters, peer values and key fingerprints | 45 |
| Wrappers, layers and sequence transforms | 21 |
| Chat transitions and persisted data model | 33 |
| Key visualization | 7 |
| API and secret-schema entities | 6 |
| Ogg voice classification and hostile framing | 41 |
| File/parser durability, excluding the nine manager cases | 13 |
| Generated schema and guarded serialization | 19 |
| Service actions and manager delivery | 61 |
| Standalone schema generation | 3 |
| Independent KDF and official-client vectors, AES backend | 12 |

## DH and chat boundaries

`DhConfig::new` accepts Telegram's byte representation. `from_parameters` additionally validates application-decoded signed generator integers. Both check the strict 2048-bit interval, generator range and documented residue conditions before the official Grammers safe-prime checker can perform modular exponentiation. Rejection reasons use fixed typed `DhFailure` variants. General small-number `is_probable_prime` and `is_safe_prime` helpers use the operating-system randomness supported by `glass_pumpkin` 2.0.0; the sieve test checks every integer below 2000 against an independent sieve. Peer checks accept inclusive `2^1984` bounds and reject negative application integers before conversion.

`Chat::for_role` initializes outbound chats as Requested and inbound chats as Waiting. Waiting has the user-facing label `pending`, preserving the established state terminology. `transition_to` enforces the legal lifecycle, and Closed is terminal even when asked to transition to Closed again. `adopt_key` returns a result, refuses terminal chats and moves Requested/Waiting to Ready. The fingerprint is derived from the stored fixed-width `SharedKey`, so no independent writable fingerprint can drift from it. `close_with_reason` accepts a closed typed reason set; error formatting never interpolates an arbitrary peer string.

The persisted model includes creation/rekey timestamps, participant metadata, counters, capabilities, exchanges and all retained queues through serde. New metadata has defaults for earlier native records. Explicit snapshots and Chat debug output omit keys, private exponents, retained bodies and ciphertext. `require_sendable` provides contextual lifecycle diagnostics; lower-level core checks retain their existing typed refusal contract.

## Independent evidence and properties

The KDF fixed cases retain unchanged Telethon reference outputs and the original independent Grammers client/server constants. Each random-direction scenario generates 100 fresh inputs and compares through Grammers' public MTProto encryption or decryption path, which exercises Grammers' own private KDF; it does not compare two calls to this project's derivation function. The 18 streaming cases compare full ciphertext bytes to one-shot Grammers AES-IGE with explicitly supplied independent random padding, and decrypt deliberately non-block-sized incoming chunks.

All 90 pre-existing independent schema vectors still run inside the published-constructor scenario. The 19 schema tests additionally check actual flags, bytes padding boundaries, legacy constructors, unknown IDs, wrong result families, invalid nested wrappers and the depth bound. Callers must run the bounded `wire::validate`/`validate_at_depth` guard before using the SDK-generated deserializer on untrusted bytes; the SDK decoder alone is not the secret-chat validation boundary.

The official-client replay discovers the archived, closed throwaway-chat captures, decrypts each frame, checks the declared body type/text/layer/counters, advances the real sequence engine and confirms the operator's reply and absence of gaps. No live client or account is used. The AES matrix case checks the declared compile-time RustCrypto backend and a published NIST AES-256 block vector through the actual IGE primitive.

The visualization renderer is isolated from key derivation and needs only Rust's standard library. Rust's byte-slice input excludes `None` and text at compilation; the two corresponding cases compile invalid calls against the actual built library and require the type error, rejecting missing-dependency setup errors. The schema privacy case likewise verifies that generated secret objects cannot be formatted through Debug or Display.

## Files, actions and generation

File save tests exercise real encrypted sources, staging files, fingerprint-before-write refusals, truncated and extended input, preserved destinations and cleanup. The interrupted-sync case writes and decrypts a real staging file, injects an I/O error at the private synchronization boundary and checks that neither the target nor plaintext temporary file survives. The public save function always supplies `File::sync_all`; no public bypass of durability was introduced.

On Unix, storage initialization records an empty, private
`.kiaro-mkdir-<digest>.pending` intent before creating each missing directory.
It synchronizes the containing directory before removing that intent. A failed
initialization leaves a recognizable intent for retry, including when the
directory already exists. Existing ancestors without an intent only need search
permission; initialization does not open unrelated ancestors for reading.

All thirteen service actions retain their closed classification, wire encoding and real manager application callback. Incoming TTL changes, optional outbound actions, retained-message self-deletion, ordered file uploads and concurrent inbound traffic use native managers with deterministic fake transports. Entity conversion accepts both API and secret-schema entities, filters unsupported negotiated layers and uses the native Grammers Markdown parser by default at the manager boundary.

`generate-schema` is a native Rust CLI. It renders the four official generator buffers into one Rust module, formats a sibling staging file and publishes atomically only after successful formatting and synchronization. `--check` compares without changing the published output; a missing output remains absent. Formatter failure preserves the earlier module and removes staging files. Cargo's build-time schema generation remains a separate disposable build-artifact step.

The general primality API was checked against the current official `glass_pumpkin` documentation: [prime::check](https://docs.rs/glass_pumpkin/2.0.0/glass_pumpkin/prime/fn.check.html) and [safe_prime::check](https://docs.rs/glass_pumpkin/2.0.0/glass_pumpkin/safe_prime/fn.check.html). Grammers' pinned source supplies the protocol checker, TL generator and independent cryptographic implementations.
