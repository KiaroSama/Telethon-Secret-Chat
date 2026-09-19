# Telethon-Secret-Chat

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
# Show chat.key_fingerprint to the user - the peer's client shows the same value.
await manager.send_message(chat.id, "hello")
```

Events are registered by name and carry a shape, never a value:

```python
manager.on("ChatReady", lambda e: print("fingerprint", e.key_fingerprint))
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
| `send_file(chat_id, path)` / `save_file(message, path)` | media |
| `set_ttl`, `mark_read`, `delete_messages`, `screenshot`, `flush_history`, `set_typing` | the chat's controls |
| `rekey(chat_id)` | a new key now, rather than on the documented trigger |

`send_message` and `send_file` resolve when **Telegram accepts the ciphertext**, not
when the peer acknowledges; acknowledgement arrives as an event.

### Running the tests

```bash
uv sync --all-extras
uv run --locked pytest tests/unit -q      # no account, no network
uv run --locked pytest tests/vectors -q   # cross-checked against Telethon's own primitives

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
uv run --locked pytest tests/interop -q -s
```

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
