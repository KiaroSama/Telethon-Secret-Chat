# Phase 1 Quickstart: proving the package works

**Date**: 2026-09-19 | **Plan**: [plan.md](./plan.md) | **Contract**: [contracts/public-api.md](./contracts/public-api.md)

Three tiers, in the order of how much they need from you. The first needs nothing, the last
needs two Telegram accounts. **The first two run in CI; the third never does.**

---

## Tier 1 — no account, no network

Everything that is a pure function: the §1.2 parameter checks, the §2 key derivation and
padding, the §3 sequence rules, the §5 action encoding. This is most of the package.

```bash
uv sync --all-extras
uv run --locked pytest tests/unit -q
```

**Expected**: every check in the [data-model](./data-model.md) validation table has a test
that supplies a FAILING input and asserts refusal. A run that passes without those is the
failure SC-003 exists to catch — the archived base package passes its own (nonexistent)
tests too.

## Tier 2 — vectors, still no account

Cross-checks the two primitives against implementations this project did not write:
Telethon's `MTProtoState._calc_key` for the key derivation and its public
`AES.encrypt_ige` for the cipher, plus any ciphertext recorded from a real chat.

```bash
uv run --locked pytest tests/vectors -q
```

**Expected**: the vendored KDF and Telethon's agree byte for byte. If they diverge, one of
them changed — and the test says which input produced the divergence.

**Note on the recorded vectors.** They are captured from a live chat with TDLib and cannot
exist before a chat works end to end (Phase 0 Q2 — TDLib exposes no crypto primitive to ask
for synthetic ones). Until step 6 of the implementation order, this tier is the two
in-process oracles only, and that is stated rather than papered over.

## Tier 3 — interop, two real accounts

The only tier that proves the thing the package claims: that a message it encrypts is read
correctly by a Telegram client it did not write.

```bash
# Both required; the tier SKIPS, loudly, when either is absent.
export TSC_TEST_SESSION="<a StringSession for the test account>"
export TSC_TEST_PEER="<the second account's username or id>"

uv run --locked pytest tests/interop -q
```

**Expected**, in order:

1. The chat is requested and the peer's real client shows it as pending.
2. On acceptance, both ends report the **same fingerprint** — compare the value the test
   prints against what the peer's client displays. This is the one check no amount of unit
   testing replaces.
3. A message sent by the package appears, correct, in the peer's client.
4. A message sent from that client is returned, correct, by the package.
5. The process is restarted and the conversation continues in order.

**Never in CI.** It needs a real authorized session, and a session string in a CI secret is
a credential one misconfigured log away from being public. The tier is for a human before a
release.

---

## The five-minute version

What using the package actually looks like, once it exists:

```python
from telethon import TelegramClient
from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.storage import FileStorage

client = TelegramClient(session, api_id, api_hash)
await client.connect()

# storage is REQUIRED - there is no default, by design
manager = SecretChatManager(client, storage=FileStorage("secret-chats.db"))
await manager.start()

chat = await manager.create(peer)
# show chat.key_fingerprint to the user; the peer's client shows the same value
await manager.send_message(chat.id, "hello")
```

## What "done" means for this feature

Not "the tests pass". The [spec's success criteria](./spec.md#success-criteria-mandatory),
of which two cannot be satisfied by any amount of unit testing:

- **SC-001** — an official client reads what this package wrote, and vice versa.
- **SC-004** — the consuming application drops its second Telegram client and the account
  shows **one** authorization.
