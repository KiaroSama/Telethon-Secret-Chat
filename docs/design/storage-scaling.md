# Design note: storage cost, a slimmer retained record, and a SQLite backend

Design spike, 2026-09-29. Measured first; the build is a follow-up feature.

## What was measured

A scratch benchmark outside the repository, on a copy of `telethon_secret_chat/storage/__init__.py`
as committed at `b67d207` (sha256 `a493dc4d...`), i.e. **before** the change to one snapshot per
outermost transaction), which landed right after. It was bounded by a
110 s internal ceiling plus a 115 s outer timeout and deleted afterwards.

One "pair" is the storage traffic of one send plus one receive in the manager: a transaction
saving the chat record and queueing one retained record (`manager._send`), then a transaction
peeking the gap queue and saving the advanced record (`manager._atomic` around
`sequence.accept`). Records have the real shape: 256-byte keys as `bytes`, a 2048-bit
`dh_prime`, retained items with a 180-byte wrapper `body` and a 264-byte `frame`, both hex.
The store was pre-filled directly (filling it through the API is itself quadratic, which is
a finding of its own: every nested call deep-copies the whole state).

Machine: Windows 11, Intel family 6 model 167, 16 logical CPUs, local SSD, Python 3.13.

| Scenario | Backend | ms per send+receive | Whole-store writes per pair | Store size |
|---|---|---|---|---|
| 5 chats × 50 retained | `MemoryStorage` | 33 | 0 | — |
| 5 chats × 50 retained | `FileStorage` | 114 | 2 | 0.32 MB |
| 50 chats × 200 retained | `MemoryStorage` | 872 | 0 | — |
| 50 chats × 200 retained | `FileStorage` | 1,512 | 2 | 10.7 MB |
| 5 chats × 1000 unacknowledged | `MemoryStorage` | 290 | 0 | — |
| 5 chats × 1000 unacknowledged | `FileStorage` | 773 | 2 | 5.3 MB |

The first full run (all three scenarios, 40 pairs each) reached the ceiling during the
third scenario; the 1000-record numbers come from a second run of 10 pairs.

## What the numbers say

- Most of the cost is not the file: `MemoryStorage` alone is 30-60 % of `FileStorage`. It
  comes from `MemoryStorage.transaction()` deep-copying and comparing the whole state on
  every entry, nested or not - since removed (one snapshot per outermost transaction).
  Re-measure on the current code.
- The file adds a full JSON dump, fsync and replace of the whole store twice per pair: about
  21 MB written per message pair at 50 × 200.
- Retained records hold both `body` and `frame` in hex, about 4× the plaintext; a silent peer
  grows them without bound (`sequence.forget_acknowledged` only drops acknowledged records).

## Threshold

The JSON store is acceptable while a send+receive pair stays under **20 ms** and writes under
**1 MB**. Before that change even the small scenario exceeds it (114 ms, 0.64 MB written); the
decision therefore waits for a measurement of the current code, repeated with this note's method.

## The consumer's load (UNVERIFIED)

Chats per account, messages per day and restart frequency are the operator's to state; they
were not available to this spike. The recommendation below is phrased so the numbers decide.

## Option 1: a slimmer retained record (independent of the backend)

Drop the plaintext `body`; store `layer`, `in_seq_no`, `out_seq_no` and `random_id`
explicitly (all `_rewrite_retained_as_deletes` needs from the wrapper), and keep `frame` as
`bytes` rather than hex. That removes plaintext at rest from the outbox and roughly halves
the record. Costs: `outbox._transmit` uses `body` equality as record identity (replace with
`(seq_no, random_id)`), and a legacy reader must accept old records (the pattern already
exists: `RetainedOutbox._retained_random_id` reads `random_id` from `body` for old stores).
Rewriting a retained message into a self-delete then needs the wrapper fields, not the body.

## Option 2: `SqliteStorage` on the standard library

- One `sqlite3` connection per store; `transaction()` maps to `BEGIN IMMEDIATE` at depth 0 and
  `SAVEPOINT` / `RELEASE` / `ROLLBACK TO` for nesting; reads inside see the open
  transaction's writes natively.
- Tables: `chats(id INTEGER PRIMARY KEY, record BLOB)`, `out(chat_id, seq_no, record BLOB,
  PRIMARY KEY (chat_id, seq_no))`, `inq(chat_id, seq_no, record BLOB, PRIMARY KEY (chat_id,
  seq_no))`. Records are JSON through the existing `FileStorage._encode` / `_decode`, so
  `bytes` and 2048-bit integers survive (a SQL `INTEGER` cannot hold them).
- `queue_out` is an upsert, `drop_out` a range delete, `queue_in` an `INSERT OR IGNORE`,
  `peek_in` a `SELECT` (no take-and-requeue), `delete` three deletes in one transaction.
- File mode owner-only on POSIX as `FileStorage` does; durability from SQLite's journal
  (`synchronous=FULL`), no whole-file replace.
- Migration: an explicit one-time import (open the JSON store with `FileStorage`, write each
  chat and both queues through `SqliteStorage` in one transaction); the application chooses
  the backend explicitly (ADR 0001), nothing switches automatically.
- Tests: add the backend to the parametrisation of `tests/unit/test_storage_contract.py`
  (the contract suite is already parametrised over the shipped backends) plus a migration test.

## Recommendation

1. Re-run this measurement on the current code. If the small scenario is under the threshold and
   the consumer runs a handful of chats, **do not build** SQLite.
2. Build option 1 (slim record) when the current file cost per pair is still above the
   threshold at the consumer's real size, or when plaintext in the outbox is judged an
   exposure on its own.
3. Build option 2 when the consumer runs more than about 20 active chats or routinely has
   more than 200 unacknowledged messages in one chat - the sizes at which the whole-store
   rewrite stays above the threshold whatever the record shape.
