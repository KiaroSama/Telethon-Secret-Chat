# Phase 0 Research: Secret chats for Telethon

**Date**: 2026-09-19 | **Plan**: [plan.md](./plan.md)

Three questions that could invalidate the plan rather than merely adjust it. Each was
answered by running something, not by reading about it. One answer changes the plan.

---

## Q1 — Is AES-IGE available without adding a cryptographic dependency?

**Decision**: use `telethon.crypto.AES.encrypt_ige` / `decrypt_ige`. No AES dependency is
added and none is written.

**Measured** (`uv run --locked python`, Telethon 1.45.0):

```
telethon.crypto.AES has: ['decrypt_ige', 'encrypt_ige']
encrypt_ige implemented with: cryptg          # falls back to pure Python when absent
pyaes 1.3.0 arrives as a Telethon dependency
```

**Rationale**: this is better than the plan assumed. `AES` is a **public** module of
Telethon — `telethon.crypto.AES`, not an underscore-prefixed internal — so using it adds
nothing to the private compatibility surface. It is also the same primitive Telethon's own
MTProto transport runs on, which means it is exercised by every Telethon user on every
connection, and it takes the `cryptg` fast path when that is installed.

**Alternatives considered**:

- *Call `pyaes` directly.* Rejected: `pyaes` has no IGE mode; Telethon implements IGE on
  top of it. Doing that ourselves is writing the cryptographic loop the constitution says
  to avoid writing.
- *Add `cryptography` or `pycryptodome`.* Rejected: both are GPL-compatible so Principle V
  would allow it, but neither is needed, and a new cryptographic dependency is exactly the
  kind of thing that must be justified rather than added for comfort.

---

## Q2 — Can TDLib produce the vectors Principle I requires?

**Decision**: **no, not synthetically — and the plan's Phase 1 assumption was wrong.**
The oracle is built from three sources instead, none of which is this package.

**Measured**: TDLib's `td_api.tl` (master, 16,313 lines) exposes **no** function that
encrypts or decrypts supplied bytes with a supplied key. Searching its function block for
any name containing `Encrypt`, `Decrypt`, `Crypto` or `Key` returns only
`setDatabaseEncryptionKey`, `getKeywordEmojis`, `setStickerKeywords`,
`getPreparedKeyboardButton` and `savePreparedKeyboardButton` — none of them a crypto
primitive. TDLib's secret-chat cryptography is entirely internal to the C++ library; the
JSON interface never reaches it.

So "capture `(key, plaintext, ciphertext)` tuples from TDLib" — which the plan listed as
the fixture source — **cannot be done through TDLib's interface at all.**

**What replaces it**, in descending order of strength:

1. **Live capture from a real secret chat.** TDLib is still installed and authorized next
   door, and it is a full Telegram client. Running a real secret chat between it and this
   package produces ciphertext TDLib actually generated, under a key both ends know from
   the exchange. Recording those exchanges gives genuine third-party vectors. This is the
   strongest oracle available and it is the one Principle I is really asking for — it just
   has to be captured from a live chat rather than requested from an API.
2. **Telethon's own `MTProtoState._calc_key` as a test oracle for the vendored KDF.**
   Measured: signature `(auth_key, msg_key, client)`, uses SHA-256, and carries the
   `x = 0 / x = 8` client/server split — the same construction §2.4 specifies. Using it in
   a **test** while not depending on it at **runtime** is the right split: the test fails
   loudly if the two ever disagree, and a Telethon release that changes the private method
   cannot change what this package does on the wire.
3. **Telethon's `AES.encrypt_ige`** for the IGE layer, per Q1 — an independently exercised
   implementation of the same primitive.

**Consequence for the plan**: the `vectors/` tier is created from a recorded live session,
not synthesised, so it cannot be built before a chat works end to end. The implementation
order is unchanged — but `crypto.py` (step 3) is verified in the first instance against
oracle 2 and 3, and the captured third-party vectors are added at step 6 when a real chat
first runs. **A task that says "pin this against TDLib" must not be scheduled before there
is a chat to capture from.**

**Alternatives considered**:

- *Published test vectors.* None exist: `core.telegram.org/api/end-to-end` specifies the
  construction and supplies no worked example.
- *Vectors from the archived base package.* Rejected outright — it is the implementation
  under suspicion, and §8 records it failing to perform most of the validation. Using it as
  an oracle would be the "two instances of the same wrong code agree" failure with an extra
  step.
- *A second Telegram account driven by an official desktop client.* Viable and stronger
  still for interop, but it cannot be automated in CI and needs a human at a keyboard. Kept
  as the acceptance check for Story 1, not as the fixture source.

---

## Q3 — Which Telethon internals are genuinely load-bearing?

**Decision**: exactly one — `TelegramClient._parse_message_text` — and it gets a canary
test plus a documented fallback. Everything else this package needs from Telethon is public.

**Measured** (Telethon 1.45.0):

| Touch point | Status | Verdict |
|---|---|---|
| `telethon.crypto.AES.encrypt_ige` / `decrypt_ige` | public | required, no canary needed |
| `messages.getDhConfig`, `requestEncryption`, `acceptEncryption`, `sendEncrypted`, `sendEncryptedFile`, `sendEncryptedService` | public TL | required |
| `InputEncryptedChat`, `EncryptedFile`, `InputEncryptedFileLocation` | public TL | required |
| `TelegramClient._parse_message_text` | **private**, present | one canary test |
| `TelegramClient._log` | **instance attribute**, not on the class | not used — this package owns its logger, which Principle IV requires anyway |
| `MTProtoState._calc_key` | private, present | **test oracle only**, never called at runtime |

**Rationale**: the base package reached into `_log` and `_parse_message_text`. `_log` is
replaced by this package's own logger — Principle IV requires control over what is emitted,
so borrowing the host's logger was never right. `_parse_message_text` remains because
formatting a message's text into entities is Telethon's job and re-implementing Markdown
and HTML parsing is not this package's business.

**Fallback if it disappears**: accept pre-parsed `entities` from the caller and skip
parsing. The canary test names that fallback so the fix is not re-derived under pressure.

**Alternatives considered**: vendoring the text parser. Rejected — it is large, it is not
protocol, and getting it subtly wrong changes what a user's message looks like rather than
whether it is secure.

---

## What changed in the plan as a result

1. **Q2 is a correction, not a confirmation.** The fixture source named in the plan does not
   exist. `vectors/` is built from a recorded live exchange and cannot precede a working
   chat; two in-process oracles cover the primitives until then.
2. **Q1 removes a risk.** AES-IGE is public API, so the compatibility surface is one private
   attribute rather than the five §8 listed.
3. **Q3 shrinks the canary surface to one test.**

No NEEDS CLARIFICATION remains.
