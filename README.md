# Telethon Secret Chat

Telegram's **MTProto 2.0 end-to-end encryption** — secret chats — for
[Telethon](https://github.com/LonamiWebs/Telethon).

Telethon never implemented it. The raw TL requests are in the schema
(`messages.requestEncryption`, `messages.sendEncrypted` and the rest) but nothing drives them:
no Diffie-Hellman exchange, no key store, no layer negotiation, no `create_secret_chat` on the
client. The project was archived in February 2026, so that gap will not close upstream.

This package fills it, so an application already built on Telethon can run secret chats on the
**same authorization it already has** — instead of carrying a second client, a second login, and
a second device row in the account's session list.

> **Status: early.** The protocol work starts from painor's archived
> [`telethon-secret-chat`](https://github.com/painor/telethon-secret-chat) (MIT), which already
> implements the exchange, AES-IGE, the MTProto 2.0 key derivation, rekeying, sequence numbers
> and file encryption — and ships no tests at all. Every inherited line is treated as unverified
> until a test has covered it. Do not use this for anything that matters yet.

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

## Usage

```python
from telethon import TelegramClient
from telethon_secret_chat import SecretChatManager, FileStorage

client = TelegramClient(session, api_id, api_hash)
await client.connect()

# Storage is REQUIRED and has no default. A library that quietly picks where to
# write key material picks a location the operator never protected.
manager = SecretChatManager(client, storage=FileStorage("secret-chats.db"))
await manager.start()

chat = await manager.create(peer)
# To verify, show chat.key_hash (36 bytes, the input to Telegram's key
# visualization) - that is what the peer's official client draws. The 64-bit
# key_fingerprint is a protocol sanity check and no client displays it.
await manager.send_message(chat.id, "hello")
```

Events are registered by name and carry a shape, never a value:

```python
manager.on("ChatReady", lambda e: show_key_visualization(e.key_hash))
manager.on("MessageReceived", lambda e: print(e.text))
manager.on("DecryptFailed", lambda e: log.warning("refused: %s", e.reason))
```

A failed decrypt is an **event**, not an exception thrown through your update loop.
Failures the protocol says must end a chat also produce `ChatClosed`.

### The operations

| Call | Does |
|---|---|
| `create(user)` / `accept(chat_id)` / `close(chat_id)` | the chat's life |
| `list()` / `status(chat_id)` | what exists, with fingerprint and TTL |
| `send_message(chat_id, text)` / `read_history(chat_id, limit)` | text |
| `send_file(chat_id, path, kind=None)` / `save_file(message, path)` | media, in any of the eight kinds |
| `set_ttl`, `mark_read`, `delete_messages`, `screenshot`, `flush_history`, `set_typing` | the chat's controls |
| `rekey(chat_id)` | a new key now, rather than on the documented trigger |

`start()` validates every stored chat before installing any. A damaged record (a key
that is not 256 bytes, a fingerprint that no longer matches its key, an unknown
state) raises `StoreCorrupt` naming the chat, and no chat is started.

`set_ttl` sends the timer to the peer, whose client enforces it. This package does
**not** delete anything locally when a TTL expires; an application that keeps
history must apply the timer itself.

A received file is written only after its key fingerprint matches the key carried
in the message. That fingerprint binds the file's key and IV to this message; it
is **not** an authentication tag over the file's bytes, and a saved file is not
proven intact by having been saved.

`send_message` and `send_file` resolve when **Telegram accepts the ciphertext**, not
when the peer acknowledges; acknowledgement arrives as an event.

### Running the tests

**`python -m pytest`, not `pytest`.** uv launches a console script through a
trampoline that cannot canonicalize a path containing a SPACE, and this project's
folder is `Telethon Secret Chat`; `uv run pytest` fails with
`uv trampoline failed to canonicalize script path`. The module form skips it. CI
runs on a path without spaces and is unaffected.

```bash
uv sync --all-extras
uv run --locked python -m pytest tests/unit -q      # no account, no network
uv run --locked python -m pytest tests/vectors -q   # cross-checked against Telethon's own primitives

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

On Windows, beside a `telegram-mcp` checkout, `scripts/run_interop.ps1` supplies the
first three from that project's `.env` without them passing through your shell:

```powershell
.\scripts\run_interop.ps1 -Account kgb_verifier -Peer "@second-account"
```

Typing a StringSession at a prompt puts a full login into shell history, scrollback
and any terminal logging you have on; this reads it into the one process that needs
it and prints nothing. It also REFUSES while the telegram-mcp server is listening,
because that server already holds the session and Telegram permanently invalidates an
auth key used from two clients at once - stop it, run this, start it again.

Each run also writes its own log to `logs/run_interop_YYYY-MM-DD_HH-mm-ss_UTC.log` under
the repository root: UTF-8, one file per run (a run in the same second gets a `_2`
suffix, so nothing is overwritten), one `[YYYY-MM-DD HH:mm:ss UTC] [LEVEL] [run_interop]
message` line per event, and the exit code on the last line. Levels are `INFO`,
`WARNING` and `ERROR`, plus `DEBUG` with `-Debug`. No session string, API hash or peer
appears in it, so the file can be attached to a support request as it is. `*.log` is
git-ignored and nothing deletes old logs; remove them when you no longer need them. If
the file cannot be created, the run says so and logs to the console only.

Accepting the chat is not one of the manual steps: the request reaches every device
the peer has and the first to complete the key exchange wins, which is normally the
phone. What still needs you is replying with the code the run prints, and opening the
files it sends.

### What is not implemented

- **MTProto 1.0.** A chat that cannot proceed without it is refused with that stated
  as the reason. Layers below 73 are out of scope.
- **Moving a chat between devices.** A secret chat is bound to the authorization that
  performed its DH exchange. Adopting this package means re-creating existing chats.
- **Two processes on one session.** That property belongs to the session, not to this
  package, and is not claimed here.

## Correctness

The rule this project is built on:

> Two instances of the same wrong code agree with each other perfectly.

So a round trip through our own encrypt/decrypt pair is **not** evidence. Every cryptographic
behaviour is pinned either by a live exchange with an official Telegram client or TDLib, or by a
fixture captured from a known-good implementation and matched byte for byte. TDLib is the oracle
while it is still present; the fixtures outlive it.

## Requirements

- Python 3.11+
- Telethon 1.45+ — the package touches exactly **one** Telethon internal
  (`TelegramClient._parse_message_text`), and `tests/unit/test_telethon_canary.py`
  fails when it disappears, naming the fallback in its failure message

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
