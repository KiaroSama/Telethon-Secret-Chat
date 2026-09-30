# Telethon Secret Chat

<div align="center">

[![Tests & Coverage](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/tests.yml/badge.svg?branch=main)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/tests.yml)
[![Lint & Format](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/python-lint-format.yml/badge.svg?branch=main)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/python-lint-format.yml)
[![Package Validation](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/package-validation.yml/badge.svg?branch=main)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/package-validation.yml)
[![CodeQL](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/workflows/codeql.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/github/license/KiaroSama/Telethon-Secret-Chat)](LICENSE)
[![Latest version](https://img.shields.io/github/v/tag/KiaroSama/Telethon-Secret-Chat?label=version&sort=semver)](CHANGELOG.md)
[![Python 3.11 | 3.12 | 3.13 | 3.14](https://img.shields.io/badge/python-3.11%20%7C%203.12%20%7C%203.13%20%7C%203.14-3776AB?logo=python&logoColor=white)](pyproject.toml)
[![Platform: Linux | Windows](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-lightgrey)](.github/workflows/tests.yml)

[![Built with Telethon 1.45+](https://img.shields.io/badge/built%20with-Telethon%201.45%2B-26A5E4?logo=telegram&logoColor=white)](https://codeberg.org/Lonami/Telethon)
[![Protocol: MTProto 2.0](https://img.shields.io/badge/protocol-MTProto%202.0-26A5E4)](docs/protocol-reference.md)
[![Code style: Black](https://img.shields.io/badge/code%20style-black-000000)](https://github.com/psf/black)
[![Top language](https://img.shields.io/github/languages/top/KiaroSama/Telethon-Secret-Chat)](https://github.com/KiaroSama/Telethon-Secret-Chat)
[![Code size](https://img.shields.io/github/languages/code-size/KiaroSama/Telethon-Secret-Chat)](https://github.com/KiaroSama/Telethon-Secret-Chat)
[![Last commit](https://img.shields.io/github/last-commit/KiaroSama/Telethon-Secret-Chat/main)](https://github.com/KiaroSama/Telethon-Secret-Chat/commits/main)
[![Open issues](https://img.shields.io/github/issues/KiaroSama/Telethon-Secret-Chat)](https://github.com/KiaroSama/Telethon-Secret-Chat/issues)
[![Support donations](https://img.shields.io/badge/Support-donations-d04a9a)](#donate)

</div>

Telegram's **MTProto 2.0 end-to-end encryption** — secret chats — for
[Telethon](https://codeberg.org/Lonami/Telethon).

Telethon never implemented it. The raw TL requests are in the schema
(`messages.requestEncryption`, `messages.sendEncrypted` and the rest) but nothing drives them:
no Diffie-Hellman exchange, no key store, no layer negotiation, no `create_secret_chat` on the
client. Telethon's development continues on [Codeberg](https://codeberg.org/Lonami/Telethon)
(1.45.0); the GitHub repository is an archived mirror since 2026-02-21, and the 1.x line still
ships no secret-chat support.

This package fills it, so an application already built on Telethon can run secret chats on the
**same authorization it already has** — instead of carrying a second client, a second login, and
a second device row in the account's session list.

> **Status: 0.2.0, in use.** [telegram-mcp](https://github.com/KiaroSama/telegram-mcp) runs its
> secret-chat tools on this package. It derives from painor's archived
> [`telethon-secret-chat`](https://github.com/painor/telethon-secret-chat) (MIT), which shipped no
> tests; this package added the evidence: the unit tier in CI on Linux and Windows, the key
> derivation checked against Telethon's own primitives, frames written by an official client
> replayed offline, and dated live runs against the official Android client — key agreement,
> messages both ways, rekey, files both ways, forwarding and the key picture (see
> [Correctness](#correctness)). Before 1.0 a minor version may still change the API; read
> [CHANGELOG.md](CHANGELOG.md) before upgrading.

## Why it exists

[telegram-mcp](https://github.com/KiaroSama/telegram-mcp) runs ~180 tools on Telethon and carried
TDLib for one reason: nine secret-chat tools. TDLib keeps its **own** authorization and cannot
import a Telethon session, so the account showed two devices for one server.

The alternatives were measured rather than assumed, and both were rejected:

| Route | Verdict |
|---|---|
| Share one auth key between Telethon and TDLib | **Impossible safely.** TDLib's `td_api.tl` has no entry point that accepts an auth key, and a shared key means two always-open MTProto connections — the documented `AUTH_KEY_DUPLICATED` condition, which invalidates the key for both. |
| Move everything to TDLib | **A rewrite.** TDLib exposes no raw TL, and the consuming server makes 200 raw `functions.*` calls across 134 request types. |

Owning the secret-chat layer in Python is the remaining route, and it is a bounded one: the
protocol has been frozen since Layer 73.

## Install

The distribution is **`kiaro-telethon-secret-chat`**; the import name is
`telethon_secret_chat`. It is not published on PyPI — the name `telethon-secret-chat` there
belongs to an unrelated package — so install it from this repository, pinned to a commit:

```bash
pip install "kiaro-telethon-secret-chat @ git+https://github.com/KiaroSama/Telethon-Secret-Chat.git@<commit>"
# Native AES for file encryption; without it files are encrypted in pure Python.
pip install "kiaro-telethon-secret-chat[fast] @ git+https://github.com/KiaroSama/Telethon-Secret-Chat.git@<commit>"
```

The `fast` extra pulls in [`cryptg`](https://pypi.org/project/cryptg/), which Telethon's AES
uses when it is installed. Changes between versions are listed in [CHANGELOG.md](CHANGELOG.md).

## Usage

```python
from telethon import TelegramClient
from telethon_secret_chat import SecretChatManager, FileStorage

client = TelegramClient(session, api_id, api_hash)
await client.connect()

# Storage is REQUIRED and has no default. A library that quietly picks where to
# write key material picks a location the operator never protected.
manager = SecretChatManager(client, storage=FileStorage("secret-chats.json"))

# Handlers BEFORE start(): see below.
manager.on("ChatReady", lambda e: ...)  # draw the key visualization from e.key_hash
manager.on("MessageReceived", lambda e: print(e.text))
manager.on("DecryptFailed", lambda e: log.warning("refused: %s", e.reason))

await manager.start()

chat = await manager.create(peer)
random_id = await manager.send_message(chat.id, "**hello**")
```

Register handlers before `start()`: it re-announces pending requests and delivers stored
messages immediately, and an event with no handler is dropped and counted as delivered.

`e.key_hash` is 36 bytes, the input to Telegram's key visualization — what the peer's
official client draws on its "Encryption Key" screen. `key_visualization(e.key_hash)` returns
the same picture as data: `rows`, 12 rows of 12 indexes into `PALETTE`, and `hex`, the 64 hex
digits shown under it (checked against the official Android client). Drawing it is the
application's job. The 64-bit `key_fingerprint` is a protocol sanity check and no client
displays it.

Text is parsed with the client's default parse mode (`client.parse_mode`, Markdown unless
changed), and the resulting entities are mapped to the secret-chat schema; entity types the
secret-chat layer cannot carry are dropped rather than sent. Pass `entities=` to skip parsing.

### Events

Registered with `manager.on(name, handler)`. Each carries a shape, never a key or a
ciphertext.

| Event | When |
|---|---|
| `ChatRequested(chat_id, peer_user_id)` | the peer asked for a chat; call `accept()` |
| `ChatReady(chat_id, peer_user_id, key_fingerprint, key_hash)` | the chat is established |
| `ChatClosedEvent(chat_id, reason)` | the chat ended, here or at the peer (`on("ChatClosed", ...)` is an accepted alias) |
| `MessageReceived(chat_id, random_id, seq_no, text, entities, ttl, media, file, reply_to, media_reference)` | a message, in conversation order; `media_reference` is set when it carries a file |
| `MessageAcknowledged(chat_id, seq_no, random_ids)` | the peer's counter passed a message this side sent |
| `ServiceActionReceived(chat_id, action_name, action, applied)` | one of the thirteen service actions, reported even when handled internally |
| `DecryptFailed(chat_id, reason)` | a received message was refused |
| `SendFailed(chat_id, random_id, cause)` | Telegram rejected a sent message for good (e.g. `DataInvalidError`); it is withdrawn as a self-delete that keeps its sequence slot, so the chat goes on |

A failed decrypt is an **event**, not an exception thrown through your update loop.
Failures that must end a chat also produce `ChatClosedEvent`: a sequence violation, a
resend this side cannot satisfy, and a message that authenticates but will not parse
(as in TDLib; skipping it would leave a hole no resend can fill).

When the PEER closes a chat, Telegram does not push that to this side in real time
(measured 2026-09-29: nothing within 60 s). `ChatClosedEvent` fires when your client next
fetches missed updates - on reconnect, or when you call `await client.catch_up()` - or on
your next send into that chat, which the server refuses and which then raises `ChatClosed`.

A synchronous handler runs before the message leaves the durable mailbox, so after a crash
it can run again (at-least-once); an asynchronous handler is scheduled, and scheduling is
not completion. Handlers own their own idempotency. Concurrent `stop()` callers share one shutdown;
cancelling one caller does not cancel the coordinator. A new `start()` does not wait for
an old file upload to finish. That old operation is rejected with `ManagerStopping` if it
returns, without replacing the restored chat or using a new sequence number.

### The operations

| Call | Returns | Does |
|---|---|---|
| `SecretChatManager(client, storage, *, history_limit=1000)` | — | `storage` is required (`StorageRequired` without it) |
| `await start()` / `await stop()` | `None` | subscribe and resume stored chats / unsubscribe, cancel handler tasks, save |
| `await create(user)` | `ChatSnapshot` | request a chat; `ChatReady` follows when the peer's device accepts |
| `await accept(chat_id)` | `ChatSnapshot` | answer a `ChatRequested`; `ChatNotReady` if there is no request |
| `await close(chat_id, reason=...)` | `None` | end the chat here and discard it on the server |
| `await forget(chat_id)` | `None` | drop a CLOSED chat's record; `list()` no longer shows it (`ValueError` if not closed) |
| `list()` / `status(chat_id)` | `ChatSnapshot` list / `ChatSnapshot` | what exists (see below) |
| `await send_message(chat_id, text, entities=None, reply_to=None)` | `random_id` | send text; `reply_to` is the `random_id` replied to |
| `await send_file(chat_id, source, *, file_name=None, caption="", mime_type=None, kind=None, reply_to=None, <media metadata>)` | `random_id` | send a path, bytes or a seekable stream as one of the eight `MEDIA_KINDS` (see below) |
| `await forward_file(chat_id, source, *, caption="", reply_to=None)` | `random_id` | send a received file (a `MessageReceived` or `MediaReference`) into any chat without downloading or uploading it |
| `await save_file(message_or_reference, path)` | `Path` | decrypt a received file and write it |
| `read_history(chat_id, limit=50)` | list of `MessageReceived` | recent messages received from the peer (see below) |
| `await set_ttl(chat_id, seconds)` | `None` | send the self-destruct timer (`ValueError` outside 0..2³¹-1) |
| `await mark_read(chat_id, random_ids)` / `await screenshot(chat_id, random_ids)` | `None` | read receipts / screenshot notice |
| `await delete_messages(chat_id, random_ids)` / `await flush_history(chat_id)` | `None` | delete here (local content first) and ask the peer to delete |
| `await set_typing(chat_id, action=None)` | `None` | typing indicator |
| `await rekey(chat_id)` | `None` | a new key now, rather than on the documented trigger |
| `await retry_pending(chat_id)` | `None` | resend anything the network did not confirm, in order; a failing record does not hold back the ones behind it. Also run at `start()` and before every send |
| `on(event, handler)` | `None` | register a handler (`ValueError` for an unknown event) |

**Media metadata.** What the peer's client shows before download comes only from what you
pass; the package reads nothing out of the file. Each argument applies to its kinds only and
anything else is refused before the upload: `duration` (seconds; video, video note, audio,
voice note), `width`/`height` (photo, video, video note, animation, sticker), `thumbnail`
(JPEG, or WEBP for a sticker, under 200 KB) with `thumbnail_size=(w, h)` each 1..320,
`waveform` (voice note, 5-bit samples, at most 63 bytes), `title`/`performer` (audio) and
`sticker_alt` (sticker). Bytes and streams need `file_name`; a stream must be seekable and
sends what remains from its position.

**Media references and forwarding.** `MessageReceived.media_reference` is a `MediaReference`:
`to_dict()` gives JSON-safe data and `MediaReference.from_dict()` rebuilds it, so a file can be
saved (`save_file`) or forwarded (`forward_file`) after a restart. A reference holds that
file's one-time key: whoever keeps it can read the file, so where it is stored is your
decision; its repr and errors never show the key. A forward is encrypted with the file's
ORIGINAL key, as official clients do (ADR 0006), so everyone who held that key can read the
forwarded copy; use `send_file` when a fresh key matters.

`list()` and `status()` return a read-only `ChatSnapshot` — `id`, `state`, `peer_user_id`,
`is_outbound`, `ttl`, `layer`, `key_fingerprint`, `key_hash`, `created_at`, `rekeyed_at`,
`closed_reason`, `has_previous_key`, `exchange_in_progress` — with no key material. Changing
it changes nothing; the operations above are the only way to change a chat.

`read_history` holds the most recent `history_limit` (default 1000) messages per chat, in
memory, received since this process started. An application that needs durable history
keeps its own.

`start()` validates every stored chat before installing any. A damaged record (a key
that is not 256 bytes, a fingerprint that no longer matches its key, an unknown
state) raises `StoreCorrupt` naming the chat, and no chat is started.

Temporary files now carry their writer process ID. Cleanup removes only owned, inactive
leftovers; it never removes another live writer’s file. Legacy temporary names without a
process ID are preserved because their ownership cannot be proved: remove them only in an
offline maintenance window after stopping **all** writers using that directory.

`FileStorage` writes the whole store through a temporary file and an atomic replace. A crash
between the two can leave a `.secret-chat-store-*.tmp` beside the store, and a crash while
saving a received file a `.secret-chat-file-*.tmp` beside it; both hold key material or
plaintext, and both are deleted the next time the store is opened or a file is saved there.
The store is JSON and **not encrypted at rest**: protect its directory and backups.

`set_ttl` sends the timer to the peer, whose client enforces it. This package does
**not** delete anything locally when a TTL expires; an application that keeps
history must apply the timer itself.

A received file is written only after its key fingerprint matches the key carried
in the message. That fingerprint binds the file's key and IV to this message; it
is **not** an authentication tag over the file's bytes, and a saved file is not
proven intact by having been saved.

### Errors

Everything raised derives from `SecretChatError`, and no error carries a key, a plaintext or
a wire object.

| Error | Raised when |
|---|---|
| `UnknownChat` | no chat with that id in this manager (also a `KeyError`) |
| `ChatNotReady` | the chat is not established, or there is no request to accept |
| `ChatClosed` | the chat is closed, or Telegram answered a send saying it no longer exists (the exception; the event is `ChatClosedEvent`) |
| `ManagerStopping` | a send while stopping, or an operation belonging to a previous `stop()`/`start()` lifecycle (also a `RuntimeError`) |
| `SendPending` | the message is stored and will be sent, but its transmission failed; carries `random_id` — **do not resend**, the next send or `start()` retries it |
| `ParameterRejected` | a Diffie-Hellman value failed a required check |
| `ResendUnsatisfiable` | the peer asked for messages this side no longer holds; the chat ends |
| `StoreCorrupt` | a stored record failed validation at `start()` |
| `StorageRequired` | the manager was built without a storage backend |
| `LayerUnsupported` | exported for API stability; not raised today (see below) |

`send_message` and `send_file` return when **Telegram accepts the ciphertext**, not when the
peer acknowledges; acknowledgement arrives as `MessageAcknowledged`.

### Running the tests

**`python -m pytest`, not `pytest`.** uv launches a console script through a
trampoline that cannot canonicalize a path containing a SPACE, and this project's
folder is `Telethon Secret Chat`; `uv run pytest` fails with
`uv trampoline failed to canonicalize script path`. The module form skips it. The same
applies to flake8, black and mypy.

```bash
uv sync --locked
uv run --locked python -m pytest tests/unit tests/vectors tests/test_packaging.py -q   # no account, no network
uv run --locked python -m flake8 . --count --show-source --statistics
uv run --locked python -m black --check .
uv run --locked python -m mypy
uv run --locked python tools/generate_schema.py --check   # the generated schema matches its source

# Interop needs two real accounts and is never run in CI. The second account
# is driven BY HAND in an official client - that is the whole claim - so the
# run prints instructions and waits for them, which is why it needs `-s`.
export TSC_TEST_SESSION="<a StringSession for the test account>"
export TSC_TEST_PEER="<the second account's username or id>"
export TSC_TEST_API_ID="<the api_id the session was created under>"
export TSC_TEST_API_HASH="<its api_hash>"
# Optional: real video/audio samples. Without it those kinds are reported as
# not covered rather than faked with a few bytes wearing a video mime type.
export TSC_TEST_MEDIA_DIR="<a directory of sample files>"
uv run --locked python -m pytest tests/interop -q -s
```

On Windows, beside a `telegram-mcp` checkout, `scripts/run_interop.ps1` (PowerShell 7) supplies
the session, API id and API hash from that project's `.env` without them passing through your
shell:

```powershell
.\scripts\run_interop.ps1 -Account kgb_verifier -Peer "@second-account"
```

| Parameter | Meaning |
|---|---|
| `-Account` | the `TELEGRAM_SESSION_STRING_<ACCOUNT>` entry to use (required) |
| `-Peer` | the second account (required) |
| `-McpRoot` | the telegram-mcp checkout holding the `.env`; default `$env:TSC_MCP_ROOT`, else a `Telegram-mcp` directory beside this repository |
| `-MediaDir` | real media samples (`TSC_TEST_MEDIA_DIR`); without it video/audio are reported as not covered |
| `-Only` | one live module or test, e.g. `tests/interop/test_live_media.py`; a subset is reported as a subset, never as the full tier |
| `-Nonce` | the code to reply with, fixed in advance (`TSC_TEST_NONCE`) so the operator knows it before the run |
| `-PeerAccount` | pair mode: the second account's `.env` entry; see the pair tier below |

Typing a StringSession at a prompt puts a full login into shell history, scrollback
and any terminal logging you have on; this reads it into the one process that needs
it and prints nothing, and pytest runs with `--tb=short` so no traceback prints fixture
arguments. It also REFUSES while the telegram-mcp server is listening, because that server
already holds the session and Telegram permanently invalidates an auth key used from two
clients at once - stop it (and its keepalive supervisor), run this, start it again.

**The pair tier** (`tests/live_pair`) puts this package on BOTH ends: two of your
accounts, the real Telegram server, no person. It checks what the offline tiers only
imitate - the server's answers, real uploads and downloads, a rekey over the real network,
a restart from the store - and it is NOT interop evidence, because two copies of the same
code agree with each other whatever they get wrong. The second account needs a public
username, and the same telegram-mcp rule applies:

```powershell
.\scripts\run_interop.ps1 -Account kgb_verifier -Peer SecondAccountUsername -PeerAccount second_account
```

Each run also writes its own log to `logs/run_interop_YYYY-MM-DD_HH-mm-ss_UTC.log` under
the repository root: UTF-8, one file per run (a run in the same second gets a `_2`
suffix, so nothing is overwritten), one `[YYYY-MM-DD HH:mm:ss UTC] [LEVEL] [run_interop]
message` line per event, and the exit code on the last line. Levels are `INFO`,
`WARNING` and `ERROR`, plus `DEBUG` with `-Debug`. No session string, API hash, peer or
nonce appears in it, so the file can be attached to a support request as it is. `*.log` is
git-ignored and nothing deletes old logs; remove them when you no longer need them. If
the file cannot be created, the run says so and logs to the console only.

Accepting the chat is not one of the manual steps: the request reaches every device
the peer has and the first to complete the key exchange wins, which is normally the
phone. Keep one device of the second account online. What still needs you is replying
with the code the run prints, and opening the files it sends. A chat that is never accepted
is closed by the run before it fails, so no pending request is left behind.

**Capturing official-client vectors.** With `TSC_CAPTURE_DIR` set (for example to
`tests/vectors/fixtures`), the round-trip test writes `official-client-<date>.json` after
its chat is closed: the chat's key — burned by that close, so nothing it protected still
exists — the direction, the raw frames the official client wrote, and what this package
read from each. No chat id, account, username or session goes in. The file is refused if
the chat is not closed. `tests/vectors/test_official_client_frames.py` replays every such
file offline (decrypt, unwrap, sequence check) and skips with its reason when there is
none. A capture is replaced by a new one from a new throwaway chat, never edited.

### What is not implemented

- **MTProto 1.0.** A peer that cannot use MTProto 2.0 (layer below 73) is out of scope;
  its messages surface as `DecryptFailed`. No refusal path is wired, by decision, so
  `LayerUnsupported` is exported but not raised.
- **Durable history.** `read_history` is in memory only (see above).
- **Telling the user the peer runs a newer layer.** The protocol suggests a notice when the
  peer's layer exceeds ours; nothing reports it.
- **Moving a chat between devices.** A secret chat is bound to the authorization that
  performed its DH exchange. Adopting this package means re-creating existing chats.
- **Two processes on one session.** That property belongs to the session, not to this
  package, and is not claimed here.

## Correctness

The rule this project is built on:

> Two instances of the same wrong code agree with each other perfectly.

So a round trip through our own encrypt/decrypt pair is **not** evidence. The evidence
that exists today is of two kinds: the interop tier, a live exchange with an official
Telegram client on real accounts that the operator runs by hand (dated runs, never in CI),
and `tests/vectors/`, which checks the key derivation against Telethon's own primitives.
A third kind joined on 2026-09-29: `tests/vectors/fixtures/official-client-2026-09-29.json`,
three frames an official mobile client wrote in a closed throwaway chat (above), replayed
offline on every CI run.

Module boundaries, the storage contract and the event contract are in
[docs/architecture.md](docs/architecture.md); the protocol itself, with the decisions this
package took where it left a choice, in [docs/protocol-reference.md](docs/protocol-reference.md).

## Requirements

- Python 3.11+
- Telethon 1.45+, below 2 — the package touches exactly **one** Telethon internal
  (`TelegramClient._parse_message_text`), and `tests/unit/test_telethon_canary.py`
  fails when it disappears, naming the fallback in its failure message
- Optional: `cryptg` (the `fast` extra) for native AES

The package ships `py.typed`; CI runs mypy in lenient mode.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).

## Donate

If this project helps you, donations are appreciated.

| Currency | Network | Address |
| --- | --- | --- |
| Bitcoin (BTC) | Bitcoin | `bc1qmth5m03pu5hujw5xw5jmywam3jj3sqwqupesdt` |
| USDT, BNB, USDC, etc. | BEP20 | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
| USDT, TRX, USDC, etc. | TRC20 | `TWBA3xFTqgZAeAYMxqo85xWnzvty3DcAhw` |
| Ethereum (ETH) | ERC20 | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
| TON | TON | `UQCN8Umo_OfOWqImZetQsrNStPcmLkMAKajFyiCOhso23NDb` |
| Litecoin (LTC) | LTC | `ltc1qntqnnrunadurnw4cshv3qgspywrueyyeyngwuy` |
| Solana (SOL) | Solana | `7B2wkczUjmkDhETwQuknBL8sUsbuV7nErxc317TmQuwR` |
| Polygon (POL) | Polygon | `0x0Bd0BA443a8B9cf15922bf7f0Bb0a4b495fD06Ef` |
