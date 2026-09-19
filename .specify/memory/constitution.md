<!--
Sync Impact Report — 2026-09-19
Version change: (none) → 1.0.0   [initial ratification]
Modified principles: none (first adoption)
Added sections:
  - Core Principles I–V
  - Secret Material Handling
  - Development Workflow
  - Governance
Removed sections: none
Deferred TODOs: none
Note: this comment is scratch for the amendment review and is removed before commit.
-->

# Telethon-Secret-Chat Constitution

## Core Principles

### I. Proved Against Another Implementation (NON-NEGOTIABLE)

Encryption correctness MUST be demonstrated against an implementation this project did not
write. Two instances of the same wrong code agree with each other perfectly, so a round trip
through our own encrypt/decrypt pair is NOT evidence of correctness and MUST NOT be presented
as such.

Every cryptographic behaviour MUST be pinned by at least one of:

1. A **live interop test** — a message exchanged with an official Telegram client or with
   TDLib, and read back correctly on the far side.
2. A **fixture from a known-good implementation** — a captured ciphertext, key fingerprint,
   or `msg_key` produced by TDLib or by published test vectors, decrypted by us and matched.

TDLib (Boost Software License 1.0) is this project's reference oracle. While the consuming
server still runs TDLib, that oracle is available in-process and MUST be used; when TDLib is
removed, captured fixtures MUST remain in the suite so the evidence outlives the oracle.

*Rationale: this is the failure mode that makes home-grown crypto dangerous. It does not
crash — it silently produces something the other side accepts, or silently weakens.*

### II. The Published Protocol Is the Specification

`https://core.telegram.org/api/end-to-end` and the TL schema are the only normative sources.
Behaviour MUST be traceable to a documented statement, not to what happened to work.

Fixed points this project is built on, and MUST NOT quietly drift from:

- Once both parties are at **Layer 73 or higher, MTProto 2.0 only**. Layer 8 and MTProto 1.0
  compatibility paths are out of scope and MUST NOT be added for convenience.
- Objects are wrapped in `decryptedMessageLayer` carrying the supported layer (46 and up).
- A secret chat is bound to the **authorization that performed the DH exchange**. It cannot be
  moved between devices, and this project MUST NOT claim or imply otherwise.

Where the documentation is silent or ambiguous, TDLib's behaviour decides, and the ambiguity
MUST be recorded in the comment at the site that implements it.

*Rationale: the secret-chat protocol has been stable for years. A deviation is a bug in us,
never an undocumented feature.*

### III. Test-First (NON-NEGOTIABLE)

Every behaviour change starts with a test that is **watched failing** on the unchanged code,
for the reason the change is about. A test written after the fix, or one never seen red, does
not satisfy this principle and MUST be rewritten.

A test MUST be able to fail for the behaviour it names. Green produced by a broad skip, a
swallowed exception, an oversized timeout, or an assertion that cannot fail is a defect in the
test, not a passing test.

*Rationale: the base this project starts from ships with no tests at all. Every line inherited
from it is unverified until a red test has covered it.*

### IV. Secret Material Never Leaves Its Boundary

Auth keys, DH parameters, secret-chat keys, key fingerprints, plaintext message bodies and
file contents MUST NOT appear in logs, exception messages, tracebacks, test fixtures,
committed files, or any string returned to a caller that did not already hold them.

A diagnostic MAY carry a **shape** — a type name, a length, a digest, a chat id — and MUST NOT
carry the value. Errors crossing the package boundary MUST be composed, never formatted from
protocol objects.

*Rationale: the point of a secret chat is that the material exists in two places and nowhere
else. A debug line is a third place.*

### V. Licence and Attribution Integrity

This project is **GPL-3.0-or-later**. It derives from `telethon-secret-chat` by painor, which
is **MIT**, and consults TDLib, which is **Boost Software License 1.0**. Both are
GPL-compatible, and both carry conditions this project MUST meet:

- The upstream MIT licence text and copyright notice MUST be preserved in `NOTICE` for as long
  as any derived line remains, and MUST NOT be dropped when a file is rewritten in place.
- TDLib is a **reference**, not a dependency. Logic ported from it MUST be marked at the site
  with the file it came from, and the Boost notice MUST appear in `NOTICE`.
- A dependency whose licence is not GPL-compatible MUST NOT be added.

*Rationale: the base is somebody else's work and the oracle is somebody else's work. Saying so
accurately is both the licence condition and the honest thing.*

## Secret Material Handling

- Key storage is the consuming application's responsibility, but this package MUST make the
  safe thing the default: a storage backend MUST be chosen explicitly, and there MUST NOT be a
  silent fallback that writes keys into the current working directory.
- Randomness MUST come from `secrets` or `os.urandom`. `random` MUST NOT appear in any path
  producing key material, nonces, or padding.
- Padding, `msg_key` derivation and sequence-number handling are specified behaviour, not
  implementation detail: each MUST carry its own test naming the documented rule it enforces.
- A received message failing any check — fingerprint, `msg_key`, sequence number, layer — MUST
  be rejected. It MUST NOT be repaired, guessed at, or delivered with a warning.

## Development Workflow

- Python, with Telethon as the transport. The package MUST work with the Telethon version the
  consuming project pins, and MUST state the minimum it supports.
- Telethon internals are a **compatibility surface**: every private attribute the package
  touches MUST be listed in one place and covered by a test that fails when it disappears.
- 800 lines is a hard ceiling per file; a file at about 700 is closed to new code and the next
  responsibility gets its own file. Generated schema files are exempt and MUST be marked as
  generated.
- The heavy test pass runs in CI on the pushed commit. Local runs stay narrow — the single
  suite being driven red, then green.
- UTF-8, LF. Comments explain **why**, carrying the measurement or the documented sentence that
  justifies the code; they MUST NOT restate what the line does.

## Governance

This constitution supersedes convenience, habit, and the practices of the code this project is
based on. Where inherited code conflicts with a principle here, the principle wins and the
inherited code changes.

**Amendment procedure.** An amendment is a pull request that changes this file, states which
principle moved and why, and carries the version bump below. An amendment weakening a
NON-NEGOTIABLE principle MUST additionally state what evidence replaces it.

**Versioning.** Semantic: MAJOR for a principle removed or redefined incompatibly, MINOR for a
principle or section added or materially widened, PATCH for clarification and wording.

**Compliance.** Every pull request MUST be reviewable against these principles, and a reviewer
MUST be able to point at the test satisfying Principles I and III for that change. Complexity
no principle requires MUST be justified in the pull request or removed.

**Version**: 1.0.0 | **Ratified**: 2026-09-19 | **Last Amended**: 2026-09-19
