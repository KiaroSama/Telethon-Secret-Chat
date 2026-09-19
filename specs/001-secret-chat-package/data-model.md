# Phase 1 Data Model: Secret chats for Telethon

**Date**: 2026-09-19 | **Plan**: [plan.md](./plan.md) | **Protocol**: `docs/protocol-reference.md`

The five entities of the spec, as concrete state. The governing question throughout is not
"what does a chat have" but **"what must survive a restart, and what must be written
atomically"** — because §4.8 says a rekey leaves two live keys and §3.7 says a gap leaves a
queue, and losing either mid-flight corrupts a conversation rather than merely interrupting it.

---

## 1. SecretChat

The conversation. One per peer chat, addressed by the id Telegram assigns.

| Field | Meaning | Survives restart |
|---|---|---|
| `id` | Telegram's secret-chat id | yes |
| `access_hash` | needed to address the chat | yes |
| `peer_user_id` | who the other end is | yes |
| `is_outbound` | did this side request the chat | yes |
| `state` | see the state machine below | yes |
| `key` | the current shared key | **yes — key material** |
| `key_fingerprint` | the 64-bit value both ends show the user | yes |
| `pending_key` | the second key during a rekey (§4) | **yes — key material** |
| `exchange_id` | identifies the rekey in flight | yes |
| `in_seq_no` | this side's counter | yes |
| `out_seq_no` | the peer's counter as last accepted | yes |
| `layer` | the layer both ends settled on (§7) | yes |
| `ttl` | self-destruct seconds, 0 = off | yes |
| `created_at`, `rekeyed_at`, `messages_since_rekey` | the §4.1 trigger inputs | yes |
| `admin_id`, `participant_id` | which side is which, for the §2.5 `x` split | yes |

**Invariant that is easy to lose:** `key` and `key_fingerprint` change together, and during
a rekey `pending_key` exists alongside `key`. A backend that persists them in separate
writes can be interrupted between the two, leaving a chat whose fingerprint does not match
its key — which decrypts nothing and looks like a peer problem. **They are one atomic unit.**

### State machine

```text
              requested ──────accept───────┐
                  │                        │
   (this side)    │                        ▼
   request ───────┤                      ready ◄──── commit ─────┐
                  │                        │  │                  │
              (peer side)                  │  └── rekeying ──────┘
   incoming ──> pending ──accept──> ready  │         │
                  │                        │         └── abort ──> ready
                  └── discard ──┐          │
                                ▼          ▼
                             closed  <── discard/peer-discard
```

- `requested` — this side sent `requestEncryption`; no key yet, nothing sendable.
- `pending` — the peer requested; this side has their public value and has not answered.
- `ready` — a key exists and both counters are live. The only state in which `send` works.
- `rekeying` — §4 exchange in flight; two keys are held. Sending still works (FR-013).
- `closed` — terminal. `send` refuses with a stated reason, never a transport error.

**Terminal means terminal.** §3.5/§3.6 name failures that must end the chat; a chat that
reached `closed` is never revived, because reviving it would mean continuing on counters
whose integrity is exactly what failed.

---

## 2. ChatKey

Not a separate stored record — a value the chat holds, given its own name because it is the
thing Principle IV protects.

| Field | Meaning |
|---|---|
| `value` | the 256-byte shared secret |
| `fingerprint` | the 64-bit identifier derived per §1.5 |
| `created_at` | when it became current, for the §4.1 age trigger |

Rules that are the package's responsibility, not the application's:

- Never included in `repr`, `str`, a log line, an exception, or any returned value.
- Never compared with `==` on the raw bytes where a timing difference could matter; the
  fingerprint is the comparable identity.
- Derived once from the DH exchange and never re-derived from stored material — a chat that
  cannot load its key is `closed`, not re-negotiated behind the application's back.

---

## 3. SecretMessage

What crosses the chat, in both directions.

| Field | Meaning | Survives restart |
|---|---|---|
| `random_id` | Telegram's deduplication id | yes, while unacknowledged |
| `seq_no` | its position, per §3.4 | yes, while unacknowledged |
| `direction` | in or out | yes |
| `payload` | text and entities, or a media reference | **outgoing only, while unacknowledged** |
| `ttl` | per-message self-destruct | yes |
| `acknowledged` | has the peer's counter passed it | yes |
| `received_at` | for the TTL countdown (§5, marked UNVERIFIED in the reference) | yes |

Two queues hang off the chat and are the reason messages are stored at all:

- **Outgoing retention** — kept until acknowledged, so §3.7's resend can be satisfied.
  Bounded; a request beyond the bound ends the chat, as the protocol requires, rather than
  being silently ignored.
- **Incoming gap queue** — messages received ahead of a hole, held in order and delivered
  only when the hole closes (FR-007). **Plaintext lives here**, so this queue is the one
  place Principle IV's boundary is most easily broken: it is never logged, never included in
  an error, and a backend that cannot hold it privately must say so rather than degrade.

---

## 4. ServiceAction

A control, not a message. Thirteen exist (§5); the package must handle all of them (FR-010)
and report rather than silently apply (FR-011).

| Group | Actions |
|---|---|
| Conversation | `SetMessageTTL`, `ReadMessages`, `DeleteMessages`, `FlushHistory`, `ScreenshotMessages`, `Typing` |
| Protocol | `NotifyLayer`, `Resend`, `Noop` |
| Rekey | `RequestKey`, `AcceptKey`, `CommitKey`, `AbortKey` |

The split matters to the design: **Conversation** actions are surfaced to the application as
events and may change stored chat state (`ttl`); **Protocol** and **Rekey** actions are the
package's own business and are handled without the application being asked, though they are
still reported so an application can log or display them.

---

## 5. StorageBackend

The interface the application supplies (FR-014). Deliberately small: the smaller it is, the
fewer ways an application can implement it wrongly.

| Operation | Contract |
|---|---|
| `load(chat_id)` | the full chat record, or nothing |
| `save(chat)` | **atomic** — key, fingerprint, pending key and counters land together or not at all |
| `delete(chat_id)` | remove it and its queues |
| `list()` | every chat id this backend holds |
| `queue_out(chat_id, message)` / `drop_out(chat_id, up_to_seq)` | outgoing retention |
| `queue_in(chat_id, message)` / `take_in(chat_id)` | the gap queue |

Written where it can be checked rather than left implied:

- **`save` is called on a meaningful state change, not on every attribute write.** §8 records
  the base package saving inside `__setattr__`, which multiplies writes and widens the window
  in which a partial write can be observed.
- **No default backend is selected.** Constructing the manager without one is an error, not a
  fallback to the working directory — FR-014 exists because a library that quietly writes key
  material somewhere is a library that writes it somewhere the operator did not protect.
- The two shipped backends are `memory` (tests) and `file` (single process, owner-only
  permissions). Neither claims to be safe for two processes; that property belongs to the
  session, not to this package.

---

## Validation rules, and where each comes from

| Rule | Source | Applied |
|---|---|---|
| `p` is a safe 2048-bit prime; `g` in range; `g`'s residue condition | §1.2 | before any key is derived |
| peer's public value in range | §1.2 | before the shared key |
| fingerprint matches what the peer shows | §1.5 | on establishment |
| padding 12–1024 bytes, 16-byte aligned | §2.2 | encrypt and decrypt |
| `msg_key` recomputed and compared | §2.3 | every receive |
| length prefix within the decrypted body | §2.7 | every receive |
| `seq_no` parity matches the sender's role | §3.4 | every receive |
| `out_seq_no` continuity: `<= C` drop, `> C+1` gap | §3.5 | every receive |
| `in_seq_no` monotonic and `<= D+1` | §3.6 | every receive |
| layer >= 73 for MTProto 2.0 | §7 | on establishment and on `NotifyLayer` |
| file key fingerprint | §6 | before any byte is written |

Every row is a test with a failing input (SC-003). A row without one counts as absent —
which is precisely what §8 found in the code this project inherits.
