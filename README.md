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

## Correctness

The rule this project is built on, stated in its
[constitution](.specify/memory/constitution.md):

> Two instances of the same wrong code agree with each other perfectly.

So a round trip through our own encrypt/decrypt pair is **not** evidence. Every cryptographic
behaviour is pinned either by a live exchange with an official Telegram client or TDLib, or by a
fixture captured from a known-good implementation and matched byte for byte. TDLib is the oracle
while it is still present; the fixtures outlive it.

## Requirements

- Python 3.11+
- Telethon 1.45+ — the package touches exactly two Telethon internals, and a test fails when
  either disappears

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).

Prior work and its conditions are recorded in [NOTICE](NOTICE): painor's MIT package, from which
this derives, and TDLib (Boost Software License 1.0), which serves as the reference
implementation. Both licences are GPL-compatible and both notices are preserved.

Not affiliated with, endorsed by, or supported by Telegram.

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
