# Feature Specification: Secret chats for Telethon

**Feature Branch**: `001-secret-chat-package`

**Created**: 2026-09-19

**Status**: Draft

**Input**: MTProto 2.0 end-to-end encryption for Telethon, written against `docs/protocol-reference.md` §1–§7 and the ratified constitution. §9 of the reference lists ten decisions the protocol leaves open; §8 shows the base package is missing DH parameter validation, sequence-number checks, and the gap/resend queue, and picks padding length with the non-cryptographic `random` module.

## User Scenarios & Testing *(mandatory)*

The "user" here is an **application built on Telethon** — in the first instance a Telegram MCP server that today carries a second Telegram client solely for secret chats. Stories are ordered so that each one, alone, is a usable product.

### User Story 1 — Hold a secret conversation (Priority: P1)

An application already signed in with Telethon starts a secret chat with a contact, exchanges text messages with them, and both sides read what the other wrote. The peer is an ordinary Telegram client — a phone, a desktop app — which does not know or care what library the other end runs.

**Why this priority**: it is the whole product. Everything else is a refinement of it, and without it nothing else has a reason to exist.

**Independent Test**: from one account, start a chat with a second real account, send a message, and read it on the receiving device; reply from the device and read it in the application. Both directions verified against a client this project did not write.

**Acceptance Scenarios**:

1. **Given** an authorized Telethon session and a contact who has a Telegram client online, **When** the application requests a secret chat, **Then** the contact's client shows the chat as pending and the application reports it as awaiting acceptance.
2. **Given** the contact accepts on their device, **When** the application looks at the chat, **Then** both ends report the SAME key fingerprint, and the application can send.
3. **Given** an established chat, **When** the application sends a text message, **Then** the peer's real client displays exactly that text.
4. **Given** an established chat, **When** the peer sends a message from their real client, **Then** the application returns exactly that text.
5. **Given** an established chat, **When** the application restarts, **Then** the chat is still usable and message ordering continues where it left off.

---

### User Story 2 — Refuse what should be refused (Priority: P1)

The application is protected from a message that fails any protocol check, from a chat whose Diffie-Hellman parameters are not safe, and from a peer who replays or reorders messages.

**Why this priority**: equal-first with Story 1 and deliberately separate. Delivering plaintext is only half the job — an end-to-end library that accepts a bad chat or a replayed message is worse than none, because it claims a guarantee it is not providing. §8 records that the base package performs **none** of these checks.

**Independent Test**: drive each rejection with a crafted input and assert the refusal; no peer needed.

**Acceptance Scenarios**:

1. **Given** a server-supplied DH configuration whose prime is not a safe 2048-bit prime, **When** the application starts or accepts a chat, **Then** it refuses and no key is created.
2. **Given** a `g` outside `1 < g < p-1`, or one failing its quadratic-residue condition for that `p`, **When** the exchange is attempted, **Then** it refuses.
3. **Given** a received message whose sequence number is one the chat has already seen, **When** it arrives, **Then** it is discarded and never delivered to the application.
4. **Given** a received message whose key fingerprint, `msg_key`, padding or layer is wrong, **When** it arrives, **Then** it is rejected, not repaired and not delivered with a warning.
5. **Given** any of the above refusals, **When** the application inspects the error, **Then** it carries the shape of the failure and **no key material, no plaintext and no protocol object**.

---

### User Story 3 — Survive a gap (Priority: P2)

Messages arrive out of order or one goes missing. The application still sees a correct, ordered conversation rather than a corrupted one.

**Why this priority**: below correctness, above conveniences. A real network reorders and drops; a library that cannot recover leaves a chat permanently broken, which in this protocol means the chat must be discarded and re-created.

**Independent Test**: deliver messages out of order and with a hole, assert the application sees them in order once the hole is filled, and assert a resend is requested.

**Acceptance Scenarios**:

1. **Given** a message arrives ahead of one that has not, **When** it is received, **Then** it is held and not delivered until the gap closes.
2. **Given** a gap persists, **When** the application decides to recover, **Then** it asks the peer to resend exactly the missing span.
3. **Given** the peer asks this application to resend a span, **When** the request arrives, **Then** the messages are re-sent once, in order, and a span that cannot be satisfied ends the chat rather than being silently ignored.

---

### User Story 4 — The conversation's own controls (Priority: P2)

Self-destruct timers, read receipts, deletions, history flush, screenshot notifications and typing indicators behave as the peer's real client expects.

**Why this priority**: what makes a secret chat a secret chat to its user rather than an encrypted pipe. Below ordering, because a wrong TTL is a privacy surprise while a wrong sequence number is a broken chat.

**Independent Test**: exercise each control against a real client and observe the effect on the far side.

**Acceptance Scenarios**:

1. **Given** the application sets a self-destruct timer, **When** the peer's client is looked at, **Then** it shows the same timer.
2. **Given** the peer sets a timer, **When** the application reports the chat, **Then** it reports the peer's value.
3. **Given** the application marks messages read, **When** the peer's client is looked at, **Then** they are shown as read.
4. **Given** the application deletes messages or flushes history, **When** the peer's client is looked at, **Then** the same messages are gone.

---

### User Story 5 — Send and receive files (Priority: P3)

Photos, video, voice and documents cross the chat and open correctly on the peer's real client.

**Why this priority**: a genuinely separate slice — text-only secret chat is already a usable product — and it carries its own key material and its own failure modes.

**Independent Test**: send each media kind to a real client and open it there; receive one from that client and verify the bytes.

**Acceptance Scenarios**:

1. **Given** a local file, **When** the application sends it, **Then** the peer's client opens it with the correct type and intact content.
2. **Given** the peer sends a file, **When** the application saves it, **Then** the saved bytes match what was sent.
3. **Given** a received file whose key fingerprint does not match, **When** it is fetched, **Then** it is refused rather than written to disk.

---

### User Story 6 — Keys that do not last forever (Priority: P3)

After enough traffic or enough time, the chat's key is replaced without the conversation stopping.

**Why this priority**: forward secrecy is what the protocol offers over a static key, and Telegram's own clients do it. Last because a chat works without it and breaking it mid-exchange is worse than delaying it.

**Independent Test**: drive the exchange over the documented trigger and assert both ends continue on the new key; assert messages issued mid-exchange are not lost.

**Acceptance Scenarios**:

1. **Given** the documented trigger is reached, **When** the application continues the conversation, **Then** a new key is negotiated and both ends keep decrypting each other.
2. **Given** an exchange is in flight, **When** the application sends, **Then** the message is delivered — not dropped and not encrypted under a key the peer has discarded.
3. **Given** the exchange fails or is aborted, **When** the application inspects the chat, **Then** it is in a stated, recoverable condition rather than silently using a half-swapped key.

### Edge Cases

- The peer never accepts: the chat stays pending and is reported as pending; no resources leak.
- The peer discards the chat from their side: the application reports it as closed, and sending refuses rather than throwing an unrelated transport error.
- The application restarts mid-exchange: on restart the chat is either usable or reported unusable — never silently on the wrong key. **Both keys and the pending queue survive a restart.**
- A message arrives for a chat this installation has no key for: refused, with nothing written to disk.
- The peer announces a layer lower than this package's: either the conversation proceeds within what both support, or the chat is refused with the reason stated — never a silent downgrade.
- A received message is MTProto 1.0: refused with that stated as the reason.
- Two application instances hold the same session: out of scope for this package, and MUST NOT be presented as supported.

## Requirements *(mandatory)*

### Functional Requirements

**Establishing a chat**

- **FR-001**: The package MUST validate every Diffie-Hellman parameter the server supplies before any key is derived — the prime's safety and length, `g`'s range, and `g`'s residue condition for that prime — and MUST refuse the chat if any check fails.
- **FR-002**: The package MUST validate the peer's public value by the same documented conditions before computing a shared key.
- **FR-003**: The package MUST expose the chat's key fingerprint so an application can show the user the same verification value the peer's client shows.
- **FR-004**: The package MUST support both starting a chat and accepting one the peer started.

**Sending and receiving**

- **FR-005**: The package MUST encrypt and decrypt messages by the documented MTProto 2.0 scheme, and MUST derive padding length from a cryptographically secure source.
- **FR-006**: The package MUST reject a received message that fails ANY of: key fingerprint, `msg_key`, padding bounds, length prefix, layer, or sequence-number validity. A rejected message MUST NOT be delivered to the application in any form.
- **FR-007**: The package MUST detect and discard a replayed message, and MUST detect a gap rather than delivering later messages as if nothing were missing.
- **FR-008**: The package MUST be able to request a resend of a missing span, and MUST answer a peer's resend request exactly once per message.
- **FR-009**: The package MUST deliver messages to the application in conversation order.

**The conversation's controls**

- **FR-010**: The package MUST support the service actions a real client expects: set TTL, read receipts, delete messages, flush history, screenshot notification, typing, layer notification, resend, and the four rekeying actions.
- **FR-011**: The package MUST report a service action received from the peer to the application rather than silently applying it.

**Files**

- **FR-012**: The package MUST send and receive encrypted files, verifying a received file's key fingerprint before the content is handed over or written anywhere.

**Keys over time**

- **FR-013**: The package MUST support key re-negotiation, and a message sent during an exchange MUST be delivered rather than dropped.
- **FR-014**: The package MUST persist everything a chat needs to survive a process restart — including both keys during an exchange and any queued messages — through a storage backend the application chooses **explicitly**. There MUST NOT be a fallback that writes key material to an unstated location.

**Boundaries**

- **FR-015**: No key material, plaintext, or protocol object may appear in a log, an exception message, a traceback, or any value returned to a caller that did not already hold it.
- **FR-016**: The package MUST state the Telethon version it supports and MUST fail a test when a Telethon internal it depends on disappears.
- **FR-017**: The package MUST NOT implement MTProto 1.0. A chat that cannot proceed without it MUST be refused with that reason stated.

### Key Entities

- **Secret chat**: one conversation with one peer. Holds its state (requested, accepted, ready, closed), the peer's identity, the current key and its fingerprint, both sequence counters, the TTL, and the layer both ends agreed on.
- **Chat key**: the shared secret and its fingerprint. During a re-negotiation, a chat holds two.
- **Message**: text or media, its sequence position, whether it has been acknowledged, and its TTL.
- **Service action**: a control the peer sent or this side is sending — a timer change, a read receipt, a deletion, a rekey step.
- **Storage backend**: where the above survives a restart. Supplied by the application; the package defines what it must be able to store and what must be written atomically.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A message sent by this package is displayed correctly by an official Telegram client, and a message sent by that client is returned correctly by this package — verified for text and for each supported media kind.
- **SC-002**: Every cryptographic step is backed by a value produced by an implementation this project did not write, and those values remain in the test suite after the reference implementation is no longer installed.
- **SC-003**: 100% of the protocol checks listed in FR-001, FR-002 and FR-006 have a test that supplies a failing input and asserts refusal. A check without such a test counts as absent.
- **SC-004**: An application can adopt the package and drop its second Telegram client, ending with **one** authorization on the account.
- **SC-005**: No key material or plaintext appears anywhere outside the chat, demonstrated by a test that exercises the failure paths and inspects what they emitted.
- **SC-006**: A chat survives an application restart and continues in order, demonstrated end to end.

## Assumptions

Ten decisions the protocol leaves open (`docs/protocol-reference.md` §9). Each is a default taken here so implementation can start; each may be revisited in `/speckit-clarify`.

- **Storage**: the application supplies the backend. The package defines the interface and ships an in-memory one for tests and a simple file one for single-process use; neither is silently selected. Persistence is per meaningful state change, not per attribute write — §8 records the base package saving on every `__setattr__`, which is both a write amplifier and a partial-write hazard.
- **API shape**: a standalone object the application constructs and owns, rather than attributes patched onto its `TelegramClient`. Patching makes the dependency invisible and unmockable.
- **Send completion**: `send` resolves when Telegram accepts the ciphertext, not when the peer acknowledges. Acknowledgement is reported separately.
- **Failure at the boundary**: a failed decrypt is a typed event delivered to the application, not an exception thrown through its update loop. Failures that the protocol says must end the chat do end it, and say so.
- **Oracle fixtures**: captured tuples of key, plaintext, padding, ciphertext and `msg_key` — not a recorded transcript. They must remain meaningful with no live account and no reference implementation present. Live interop tests are opt-in and skipped when no test account is configured.
- **MTProto 1.0**: refused, per FR-017.
- **Layer**: announce the highest layer whose constructors this package can parse, and never a higher one. The number is set once there is code to justify it.
- **Telethon compatibility**: the key-derivation routine is implemented here rather than borrowed from a Telethon private method. It is about ten lines and fully specified; depending on a private method for a cryptographic step means a silent behaviour change on the next release. The remaining touch points are listed in one place and pinned by a canary test.
- **Rekey policy**: the package rekeys automatically on the documented trigger, and the application can ask for one. It is not left to the caller to remember.
- **Resend budget**: adopt the reference implementation's span cap rather than inventing a number; retention of outgoing messages is bounded and stated, and a request beyond it ends the chat as the protocol requires.

Other assumptions:

- Python 3.11+, Telethon 1.45+ (the layer floor already recorded in `pyproject.toml`).
- Only ONE application instance uses a given session at a time. Concurrent use is a property of the session, not of this package.
- The first consumer is an MCP server whose existing secret-chat tools define the API this must be able to serve: create, close, list, status, set timer, send message, send media, read messages, save media.
- Existing chats held by another client **cannot** be migrated. They are bound to the authorization that performed their exchange; adopting this package means re-creating them.
