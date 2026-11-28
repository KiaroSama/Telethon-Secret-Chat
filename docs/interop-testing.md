# Interoperability testing

Use **Actions > Live Interoperability (Opt-In) > Run workflow** to choose a tier. The live tiers create a temporary secret chat on Telegram. Use only disposable accounts you control. Open a regular Telegram dialog between the two users first, and keep it among account A's 100 most recent dialogs.

## Offline vectors

The default CI suite needs no credentials. It covers MTProto framing and encryption, DH validation, sequence counters, PFS rekey, replay handling, and a simulated exchange between two library instances. Choose `offline-vectors` to run a subset manually. These tests do not contact Telegram.

## Same-library pair

Choose `same-library-pair` to run both ends with Grammers Secret Chat. The harness establishes a secret chat, compares the two derived key visualizations, and sends a challenge and reply in opposite directions. It only passes if both ends decrypt the expected text. No one needs to operate a client during this run.

## Official client

Choose `official-client-interop` to run account A with Grammers Secret Chat and account B with an official Telegram client that supports Secret Chats. Keep B's client open. Accept the new secret chat, then reply inside that chat with the code from the message. Send only the code. You have five minutes after the request; silence, a wrong reply, or a connection error cannot pass the test.

This tier verifies establishment and text in both directions. It does not test media, TTL, read receipts, or rekeying. The harness tries to close the temporary chat without deleting history, even after a failed exchange. If the job reports that cleanup failed or no request ID was confirmed, check the controlled accounts for a leftover chat.

## Credentials and local runs

Provide these as GitHub Actions Secrets or through private local environment variables. Do not paste values into the workflow dispatch form or commit them.

| Variable | Value |
| --- | --- |
| `TELEGRAM_INTEROP_API_ID` | Positive numeric ID for the dedicated test application. |
| `TELEGRAM_INTEROP_API_HASH` | Its 32-digit hexadecimal API hash. The harness checks the format; it does not sign in or send the hash. |
| `TELEGRAM_INTEROP_SESSION_A` | Already authorized test session as `<home DC>:<256-byte key as 512 hex digits>`, with a known production DC from 1 to 5. |
| `TELEGRAM_INTEROP_SESSION_B` | A separate authorization in the same format. Required for pair mode; leave unset for official-client mode. |
| `TELEGRAM_INTEROP_PEER_USER_ID` | The peer's Telegram user ID. Required for official-client mode; optional in pair mode, where it must match account B. |

A session value grants account access. Use a dedicated authorization, never a personal session. Rotate or revoke it if it leaves private storage. The harness stores it in memory for the run. Errors report the failed step without printing credential values, message text, or account identifiers.

For a local run, load the variables privately, then run one test:

```sh
cargo test --locked --test live_interop_harness -- --ignored --nocapture same_library_pair --exact
cargo test --locked --test live_interop_harness -- --ignored --nocapture official_client_interop --exact
```

Ordinary `cargo test` checks credential validation without opening a connection. It skips both live tests.
