# Tasks: Secret chats for Telethon

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Protocol**: `docs/protocol-reference.md`

## Format: `[ID] [P?] [Story] Description`

- **[P]** — parallelizable: different file, no dependency on an incomplete task.
- **[US1]…[US6]** — the user story from the spec this task serves.

## Path Conventions

Package at `telethon_secret_chat/`, tests at `tests/{unit,vectors,interop}/`. Every module
is named for the protocol-reference section it implements, so a reviewer can put `§3.4`
beside `sequence.py`.

**Tests ARE requested and are not optional here.** Constitution Principle III makes a
watched-red test the precondition for writing code, and Principle I makes an outside oracle
the precondition for believing it. Every implementation task below names the test that must
be red first.

**The ordering constraint from research.md Q2:** TDLib exposes no crypto primitive, so
recorded third-party vectors cannot exist until a real chat runs. Tasks that capture them
are in Phase 8 and **must not be pulled earlier** — until then the oracles are Telethon's
`MTProtoState._calc_key` and `AES.encrypt_ige`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: the skeleton every later phase writes into.

- [X] T001 Create the package layout from plan.md in `telethon_secret_chat/` — empty modules `errors.py`, `dh.py`, `handshake.py`, `crypto.py`, `framing.py`, `sequence.py`, `rekey.py`, `actions.py`, `files.py`, `chat.py`, `manager.py`, and packages `storage/` and `schema/`, each with a docstring naming the protocol-reference section it implements
- [x] T002 [P] Create `tests/unit/`, `tests/vectors/`, `tests/interop/` with `__init__.py` and a `conftest.py` that makes `tests/interop` skip with a stated reason when `TSC_TEST_SESSION` or `TSC_TEST_PEER` is unset
- [x] T003 [P] Add the coverage source list to `pyproject.toml` — one entry per module, so a new file must be added deliberately and cannot arrive uncovered
- [X] T004 Generate the secret-chat TL schema into `telethon_secret_chat/schema/secret_tl.py`, marked `# generated` at the top and exempted from the line ceiling in `.flake8`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: the two things every later task depends on. **No user story can start until these are done.**

- [x] T005 Write `tests/unit/test_errors_leak_nothing.py`: for every error type in the contract, assert `str(e)` and `repr(e)` contain the error name and chat id and NOT a key, plaintext, ciphertext or protocol-object repr. Watch it fail against no implementation
- [x] T006 Implement `telethon_secret_chat/errors.py` — `SecretChatError` and the six subtypes from `contracts/public-api.md` §4, each composing its message from a shape, never formatting a protocol object. T005 goes green
- [x] T007 [P] Implement `telethon_secret_chat/storage/__init__.py` — the `StorageBackend` interface from data-model.md §5, and `storage/memory.py`. `save` is atomic across key, fingerprint, pending key and counters **as one unit**; constructing a manager without a backend raises `StorageRequired`
- [x] T008 [P] Write `tests/unit/test_storage_contract.py` — a shared suite any backend must pass, including: a `save` interrupted between key and fingerprint leaves the PREVIOUS consistent record, never a mixed one

---

## Phase 3: User Story 2 — Refuse what should be refused (Priority: P1) 🎯 MVP-critical

**Goal**: a chat is never established on parameters the protocol says to reject, and no
malformed message is ever delivered.

**Independent test**: every check driven by a crafted failing input; no peer, no account.

**Why this is first**: §8 measured the inherited code performing NONE of these. A chat
established on a bad prime cannot be repaired afterwards, so this precedes the happy path.

### Tests for User Story 2 (write first, watch fail)

- [X] T009 [P] [US2] `tests/unit/test_dh_parameters.py` — §1.2: a `p` that is not a safe 2048-bit prime, a `p` of the wrong bit length, `g` outside `1 < g < p-1`, and a `g` failing its quadratic-residue condition for that `p`. Each asserts `ParameterRejected` and that **no key was derived**
- [X] T010 [P] [US2] `tests/unit/test_peer_value.py` — §1.2: the peer's `g_a`/`g_b` outside the documented range is rejected before the shared key is computed
- [X] T011 [P] [US2] `tests/unit/test_receive_rejections.py` — §2.7: wrong key fingerprint, wrong `msg_key`, padding outside 12–1024 or not 16-byte aligned, and a length prefix beyond the decrypted body. Each asserts rejection and that **nothing was delivered**

### Implementation for User Story 2

- [X] T012 [US2] Implement `telethon_secret_chat/dh.py` — §1.2 safe-prime test, bit length, `g` range, and the `p mod 8 / 3 / 5 / 24 / 7` residue conditions, citing the section at each check. T009 and T010 go green
- [X] T013 [US2] Implement the receive-side validation in `telethon_secret_chat/crypto.py` — fingerprint, recomputed `msg_key`, padding bounds, length prefix. T011 goes green

**Checkpoint**: every §1.2 and §2.7 rule has a failing-input test. SC-003 is satisfiable for this set.

---

## Phase 4: User Story 1 — Hold a secret conversation (Priority: P1) 🎯 MVP

**Goal**: start a chat, exchange text with a real client, survive a restart.

**Independent test**: with a second real account, send and read in both directions.

### Tests for User Story 1 (write first, watch fail)

- [X] T014 [P] [US1] `tests/vectors/test_kdf_matches_telethon.py` — the vendored §2.4 derivation against Telethon's `MTProtoState._calc_key(auth_key, msg_key, client)` for both `x = 0` and `x = 8`. **This is the oracle, not a self-check**
- [X] T015 [P] [US1] `tests/unit/test_padding.py` — §2.2: length is 12–1024, total is 16-byte aligned, and the source is `secrets`/`os.urandom`. A test asserts the `random` module is not imported anywhere in the package
- [ ] T016 [P] [US1] `tests/unit/test_framing.py` — §3.1–§3.3: `decryptedMessageLayer` wrapping, `layer_no` >= 46, `random_bytes` minimum length
- [ ] T017 [P] [US1] `tests/unit/test_fingerprint.py` — §1.5: the fingerprint of a known shared key, cross-checked against the value computed from the reference's stated construction
- [ ] T018 [US1] `tests/unit/test_chat_state.py` — the data-model.md state machine: which transitions are legal, that `closed` is terminal, and that `send` refuses outside `ready`/`rekeying` with `ChatNotReady`
- [ ] T019 [US1] `tests/unit/test_restart.py` — a chat saved, the manager destroyed, a new manager loaded from the same backend: same key, same fingerprint, same counters, and the conversation continues in order

### Implementation for User Story 1

- [ ] T020 [US1] Implement `telethon_secret_chat/crypto.py` §2.1–§2.6 — serialization, padding from `secrets`, `msg_key`, the vendored KDF, and the IGE frame via the PUBLIC `telethon.crypto.AES`. T014, T015 go green
- [ ] T021 [US1] Implement `telethon_secret_chat/framing.py` — §3.1–§3.3. T016 goes green
- [ ] T022 [US1] Implement `telethon_secret_chat/handshake.py` — §1.3–§1.6 request, accept, shared key, fingerprint, using `dh.py` for every parameter. T017 goes green
- [ ] T023 [US1] Implement `telethon_secret_chat/chat.py` — the `SecretChat` entity and the state machine from data-model.md §1, with `key` and `key_fingerprint` written as ONE atomic unit. T018 goes green
- [ ] T024 [US1] Implement `telethon_secret_chat/storage/file.py` — the single-file backend, owner-only permissions, atomic replace. T008's shared suite goes green for it
- [ ] T025 [US1] Implement `telethon_secret_chat/manager.py` — construction per `contracts/public-api.md` §1, `start`/`stop`, the update subscription, and `create`/`accept`/`close`/`list`/`status`. T019 goes green
- [ ] T026 [US1] Implement `send_message` and `read_history` in `telethon_secret_chat/manager.py`, resolving on Telegram's acceptance per the spec's Assumptions, and the `MessageReceived` / `ChatReady` / `ChatClosed` events
- [ ] T027 [US1] Export exactly the contract surface from `telethon_secret_chat/__init__.py` — and nothing else. A test asserts `__all__` matches the contract and that no protocol internal is reachable from the package root

**Checkpoint**: 🎯 **MVP.** A text conversation works and survives a restart. Stop here and it is a usable product.

---

## Phase 5: User Story 3 — Survive a gap (Priority: P2)

**Goal**: reordering and loss produce a correct ordered conversation, not a corrupt one.

**Independent test**: deliver messages out of order and with a hole; assert ordering and a resend request.

### Tests for User Story 3 (write first, watch fail)

- [ ] T028 [P] [US3] `tests/unit/test_sequence_parity.py` — §3.4: a message whose `seq_no` parity does not match the sender's role is rejected and the chat ends, per the reference
- [ ] T029 [P] [US3] `tests/unit/test_replay_and_gap.py` — §3.5: `out_seq_no <= C` is DISCARDED and never delivered; `> C+1` is held as a gap, not delivered; the hole closing releases the held messages **in order**
- [ ] T030 [P] [US3] `tests/unit/test_in_seq_no.py` — §3.6: the peer's echo of this side's counter must be monotonic and `<= D+1`; a violation ends the chat
- [ ] T031 [US3] `tests/unit/test_resend.py` — §3.7: a gap produces a `Resend` for exactly the missing span; an incoming `Resend` is answered once per message, in order; a span outside retention raises `ResendUnsatisfiable` and ends the chat

### Implementation for User Story 3

- [ ] T032 [US3] Implement `telethon_secret_chat/sequence.py` — §3.4–§3.8 parity, replay, gap detection, the ordered gap queue, and outgoing retention with its stated bound. T028–T031 go green
- [ ] T033 [US3] Wire the gap queue and retention through `telethon_secret_chat/storage/__init__.py` and `storage/file.py` so both survive a restart, extending `tests/unit/test_storage_contract.py`

**Checkpoint**: a lossy, reordering network no longer corrupts a conversation.

---

## Phase 6: User Story 4 — The conversation's own controls (Priority: P2)

**Goal**: TTL, read receipts, deletion, flush, screenshots and typing behave as a real client expects.

**Independent test**: exercise each against a real client and observe the far side.

- [ ] T034 [P] [US4] `tests/unit/test_actions_encoding.py` — all thirteen `decryptedMessageAction*` from §5 round-trip, and an unknown action is reported rather than dropped
- [ ] T035 [US4] Implement `telethon_secret_chat/actions.py` — §5, splitting Conversation actions (surfaced as `ServiceActionReceived`, may change stored `ttl`) from Protocol/Rekey actions (handled internally, still reported). T034 goes green
- [ ] T036 [US4] Add `set_ttl` to `telethon_secret_chat/manager.py` and the TTL field to `telethon_secret_chat/chat.py`, noting at the site that the countdown start is UNVERIFIED in the reference and that this package stores and transmits without enforcing locally

---

## Phase 7: User Story 5 — Send and receive files (Priority: P3)

**Goal**: media crosses the chat and opens on the peer's real client.

- [ ] T037 [P] [US5] `tests/unit/test_file_keys.py` — §6: a file whose key fingerprint does not match is refused **before any byte is written to disk**
- [ ] T038 [US5] Implement `telethon_secret_chat/files.py` — §6 key/iv/fingerprint, encrypted upload and download. T037 goes green
- [ ] T039 [US5] Add `send_file` and `save_file` to `telethon_secret_chat/manager.py` per `contracts/public-api.md` §2

---

## Phase 8: Third-party vectors and interop 🔒 BLOCKED until Phase 4 is green

**Purpose**: Principle I's strongest evidence. **research.md Q2: TDLib exposes no crypto
primitive, so these cannot be synthesised — they are recorded from a real chat. Do not
attempt these before a chat works.**

- [ ] T040 [US1] Record a real secret-chat exchange with TDLib: capture the negotiated key, the plaintexts and the ciphertexts TDLib produced, into `tests/vectors/recorded/` as fixtures that carry no account identity
- [ ] T041 [P] [US1] `tests/vectors/test_recorded_tdlib.py` — decrypt each recorded TDLib ciphertext with the recorded key and assert the plaintext; encrypt each recorded plaintext and assert the ciphertext matches byte for byte
- [ ] T042 [US1] `tests/interop/test_live_roundtrip.py` — opt-in: request a chat with a second real account, assert both ends report the SAME fingerprint, send and read in both directions (SC-001)
- [ ] T043 [US5] `tests/interop/test_live_media.py` — opt-in: each media kind sent and opened on the far side, and one received and verified

---

## Phase 9: User Story 6 — Keys that do not last forever (Priority: P3)

- [ ] T044 [P] [US6] `tests/unit/test_rekey.py` — §4: the 100-message / one-week trigger, the four-action exchange, and that a message sent MID-EXCHANGE is delivered rather than dropped or encrypted under a discarded key
- [ ] T045 [US6] Implement `telethon_secret_chat/rekey.py` — §4, holding both keys during the exchange and persisting both. T044 goes green
- [ ] T046 [US6] Extend `tests/unit/test_restart.py`: a restart DURING an exchange leaves the chat usable or stated-unusable, never silently on a half-swapped key

---

## Phase 10: Polish & Cross-Cutting Concerns

- [ ] T047 [P] `tests/unit/test_telethon_canary.py` — asserts `TelegramClient._parse_message_text` exists, names the documented fallback in its failure message, and asserts `telethon.crypto.AES.encrypt_ige` is still public
- [ ] T048 [P] Extend `tests/unit/test_errors_leak_nothing.py` to drive EVERY failure path added in Phases 3–9 and assert the same boundary (SC-005)
- [ ] T049 [P] Add a `tests/unit/test_no_insecure_random.py` that fails if `import random` appears anywhere in the package
- [ ] T050 Write the README usage section from `quickstart.md`, and record in `.ai/memory.md` which of the reference's UNVERIFIED items remain unverified
- [ ] T051 Re-run `graphify .` and re-index CBM against `telethon_secret_chat/` now that there is real code to graph

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 → Phase 2** — the skeleton before the foundations.
- **Phase 2 blocks everything.** Errors and storage are what every other phase raises and persists through.
- **Phase 3 (US2) before Phase 4 (US1)** — deliberately inverted against the spec's story order. Validation precedes the happy path because a chat established on bad parameters cannot be fixed later, and because the inherited code's happy path is exactly what tempts a paste.
- **Phase 4 blocks Phases 5, 6, 7, 8, 9** — they all need a working chat.
- **Phase 8 is hard-blocked by Phase 4** by research.md Q2. It is the one ordering that is a fact about TDLib, not a preference.

### User Story Dependencies

- **US2** (refusals) — independent; needs only Phase 2.
- **US1** (conversation) — needs US2's `dh.py`, since establishment validates.
- **US3** (gaps), **US4** (controls), **US5** (files), **US6** (rekey) — each needs US1 and none needs another. They can proceed in parallel once Phase 4 is green.

### Parallel Opportunities

- T002, T003 in Phase 1.
- T007, T008 in Phase 2.
- T009, T010, T011 — all three US2 test files.
- T014–T017 — the four US1 test files.
- T028, T029, T030 — the sequence test files.
- **Phases 5, 6, 7 and 9 are four independent tracks** once Phase 4 is green.

## Parallel Example: User Story 2

```text
# The three failing-input suites are independent files:
T009 tests/unit/test_dh_parameters.py
T010 tests/unit/test_peer_value.py
T011 tests/unit/test_receive_rejections.py
# then, serially, because they share crypto.py:
T012 dh.py  →  T013 crypto.py receive-side
```

## Implementation Strategy

### MVP First

Phases 1 → 2 → 3 → 4. That is **27 tasks** and delivers: a secret chat that establishes
only on valid parameters, exchanges text with a real Telegram client, refuses every
malformed message, and survives a restart. Everything after is a slice.

### Incremental Delivery

Each of Phases 5–7 and 9 is shippable alone. Phase 8 is not a feature — it is the evidence
for Phase 4, and it is why TDLib stays installed next door until it is done.

### The one thing not to do

Do not port a function from the archived base package into a module and then write the test.
§8 records that package failing most of the validation this feature exists to add; a test
written after a paste goes green without ever having been red, and Principle III exists
precisely because that is the easy mistake here.

## Notes

- **51 tasks**: Setup 4, Foundational 4, US2 5, US1 14, US3 6, US4 3, US5 3, vectors/interop 4, US6 3, Polish 5.
- TDLib is removed from the consuming project only after T042 passes — not after T027.
- Every task names a file. Every implementation task names the test that must be red first.
