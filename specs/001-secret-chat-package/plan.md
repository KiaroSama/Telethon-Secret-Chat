# Implementation Plan: Secret chats for Telethon

**Branch**: `001-secret-chat-package` | **Date**: 2026-09-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/001-secret-chat-package/spec.md`

## Summary

Implement Telegram's MTProto 2.0 end-to-end encryption as a Python package Telethon
applications can own, so an application already holding one authorization stops needing a
second Telegram client for secret chats.

The protocol is specified and frozen; the work is **validation and evidence**, not
discovery. `docs/protocol-reference.md` §8 measured the archived base package against the
published protocol: the exchange, AES-IGE, the MTProto 2.0 KDF, rekeying and file
encryption are present, while every parameter check, every sequence-number check, the
gap/resend queue and the padding's randomness source are absent. So the approach is to
build the checked path first and let the inherited code serve as a reference for shape —
never as a trusted implementation.

## Technical Context

**Language/Version**: Python 3.11+ (3.11, 3.12, 3.13, 3.14 in CI)

**Primary Dependencies**: Telethon 1.45+ (transport and TL schema only). `pyaes` arrives
through Telethon and provides AES-IGE. No cryptographic dependency is added.

**Storage**: application-supplied backend behind an interface this package defines. Two
shipped: in-memory (tests) and a single-file one (single-process use). Neither is
selected silently — FR-014.

**Testing**: pytest, with three tiers — unit (pure functions), vector (fixtures captured
from TDLib), and interop (a live account, opt-in, skipped when unconfigured).

**Target Platform**: any platform Telethon runs on; developed and CI-tested on Linux and
Windows.

**Project Type**: library

**Performance Goals**: not a performance feature. The only budget that matters: a message
round trip must not add latency a user notices over an ordinary Telethon send, and a
restart must not re-derive keys.

**Constraints**: no key material or plaintext outside the chat (Principle IV); 800-line
file ceiling; generated schema exempt and marked; correctness demonstrated against an
implementation this project did not write (Principle I).

**Scale/Scope**: tens of concurrent secret chats per application, single process. Nine
consumer operations define the API surface: create, close, list, status, set timer, send
message, send media, read messages, save media.

## Constitution Check

*GATE: passed before Phase 0. Re-checked after Phase 1 design — see below.*

| Principle | How this plan satisfies it | Gate |
|---|---|---|
| **I. Proved against another implementation** | Three oracles, none of them this package: Telethon's `MTProtoState._calc_key` for the KDF and its public `AES.encrypt_ige` for the cipher, used in tests only; and ciphertext captured from a REAL chat with TDLib, recorded at step 6. Phase 0 established that TDLib cannot be asked for synthetic vectors — see [research.md](./research.md) Q2 — so the captured tier follows the first working chat instead of preceding it. Interop tests are opt-in. | PASS |
| **II. The published protocol is the specification** | Every module cites the reference section it implements (`§2.3`, `§3.4`, …). Where the reference says UNVERIFIED, the code says so at the site and the spec's Assumptions name the default taken. | PASS |
| **III. Test-first** | Each task below is stated as "test watched red, then code". The inherited implementation makes this easy to violate — a paste makes a test green without it ever having been red — so tasks name the failing input first. | PASS |
| **IV. Secret material never leaves its boundary** | One error type carrying a shape only; a test that drives every failure path and asserts the emitted text contains no key, no plaintext and no protocol repr. Introduced with the first failure path, not retrofitted. | PASS |
| **V. Licence and attribution integrity** | `NOTICE` carries painor's MIT and TDLib's Boost; a tracked test already pins both. Any logic ported from TDLib names the file at the site. | PASS |

No violations. Complexity Tracking is therefore empty and omitted.

**Post-Phase-1 re-check**: the design below adds no component that a principle does not
require. The one judgement worth restating is vendoring the key derivation (~10 lines,
fully specified in §2.4) rather than calling Telethon's private `MTProtoState._calc_key`:
Principle I says correctness must be provable, and a private method can change on any
release without notice. That is a deliberate ~10 lines, and it gets a vector test.

## Project Structure

### Documentation (this feature)

```text
specs/001-secret-chat-package/
├── plan.md              # This file
├── spec.md              # The specification
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output — the package's public surface
└── checklists/
    └── requirements.md  # Spec quality checklist
```

### Source Code (repository root)

```text
telethon_secret_chat/
├── __init__.py          # the public surface, and nothing else
├── errors.py            # the error types; shapes only, never values (Principle IV)
├── dh.py                # §1.2 parameter validation — safe prime, g range, g residue
├── handshake.py         # §1.3-§1.6 request/accept, shared key, fingerprint
├── crypto.py            # §2 KDF, padding, msg_key, AES-IGE frame
├── framing.py           # §3.1-§3.3 decryptedMessageLayer, layer, random_bytes
├── sequence.py          # §3.4-§3.8 seq numbers, replay, gap, resend bookkeeping
├── rekey.py             # §4 PFS exchange
├── actions.py           # §5 service actions
├── files.py             # §6 encrypted files
├── chat.py              # the SecretChat entity and its state machine
├── manager.py           # the object an application owns; subscribes to updates
├── storage/
│   ├── __init__.py      # the backend interface
│   ├── memory.py        # in-memory, for tests
│   └── file.py          # single-file, for single-process use
└── schema/
    └── secret_tl.py     # GENERATED TL for the secret-chat layer; marked generated

tests/
├── test_packaging.py    # exists: licence conditions
├── unit/                # pure functions, no network, no account
├── vectors/             # fixtures captured from TDLib + the tests that consume them
└── interop/             # live account; opt-in, skipped when unconfigured
```

**Structure Decision**: one module per protocol responsibility, named for the reference
section it implements, because that is what makes Principle II checkable — a reviewer can
put `§3.4` beside `sequence.py` and compare. The alternative, one `secret_methods.py` of
660 lines as the base package has, is what allowed a `# TODO add checks` to sit
unnoticed in the middle of the sequence-number handling for six years.

`schema/secret_tl.py` is generated and exempt from the 800-line ceiling, marked as such
at the top of the file.

## Phase 0 — Research

Output: [research.md](./research.md). Every §9 open question already has a default in the
spec's Assumptions; Phase 0's job is to confirm the three that could invalidate the plan
rather than merely adjust it:

1. **Is AES-IGE available without a new cryptographic dependency?** ANSWERED: yes, and
   better than assumed — `telethon.crypto.AES` is PUBLIC API with a `cryptg` fast path, so
   it adds nothing to the private compatibility surface.
2. **Can TDLib emit the vectors Principle I needs?** ANSWERED: **no.** Its `td_api.tl`
   exposes no crypto primitive at all; the secret-chat cryptography never reaches the JSON
   interface. This corrected the plan — see the Constitution Check row above and the
   implementation order below.
3. **Which Telethon internals are load-bearing?** ANSWERED: exactly one,
   `_parse_message_text`, with a named fallback. `_log` is not used at all: Principle IV
   requires this package to control what it emits, so it owns its logger.

## Phase 1 — Design & Contracts

Outputs: [data-model.md](./data-model.md), [contracts/](./contracts/),
[quickstart.md](./quickstart.md).

- **data-model.md** — the five entities from the spec as concrete state: what a chat holds,
  what survives a restart, what must be written atomically, and the chat's state machine
  including the two-key window during a rekey.
- **contracts/** — the package's public surface: what an application constructs, the nine
  operations the first consumer needs, the events it receives, and the storage interface it
  must satisfy. The contract is what `__init__.py` exports and nothing more.
- **quickstart.md** — the runnable proof: set up a storage backend, start a chat with a
  real account, send and read, and the exact commands, including how to run the vector tier
  with no account at all.

## Implementation order

Derived from the spec's priorities, and arranged so the first thing built is the thing the
base package lacks:

1. **Errors and the no-leak test** (Principle IV) — everything else raises through it.
2. **`dh.py`** — the parameter checks, driven by failing inputs. This is Story 2 and it is
   first because a chat established on a bad prime is not fixable later.
3. **`crypto.py`** against captured vectors — KDF, padding from `secrets`, `msg_key`, IGE.
4. **`framing.py` + `handshake.py`** — up to a computed fingerprint that matches a real
   client's.
5. **`sequence.py`** — the parity, replay and gap rules; Story 3's queue.
6. **`chat.py` + `manager.py` + `storage/`** — Story 1 end to end, restart included.
7. **`actions.py`** — Story 4.
8. **`files.py`** — Story 5.
9. **`rekey.py`** — Story 6.

Steps 1–6 are a usable product; 7–9 each add a slice. TDLib stays installed next door
through all of them, and is removed from the consuming project only after step 6 is proved
against a real client.

**One ordering constraint comes out of Phase 0 Q2 and must not be lost:** the third-party
vector tier is captured from a real chat with TDLib, so it is created AT step 6, not before.
Steps 3–5 are verified against the two in-process oracles; a task that says "pin this against
TDLib" cannot be scheduled earlier than step 6, because there is nothing to capture from.
