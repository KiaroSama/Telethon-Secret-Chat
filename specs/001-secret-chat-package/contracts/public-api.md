# Contract: the package's public surface

**Date**: 2026-09-19 | **Plan**: [../plan.md](../plan.md)

What `telethon_secret_chat/__init__.py` exports, and nothing more. Everything else is
internal and may move without notice.

The rule this contract is written under: **the package is an object the application owns**,
not attributes patched onto its `TelegramClient`. Patching makes the dependency invisible to
a reader and unmockable to a test — and the archived base package did exactly that.

---

## 1. Construction

```
SecretChatManager(client, storage)
```

| Parameter | Contract |
|---|---|
| `client` | a connected, authorized Telethon client. The manager does not connect it, does not log it in, and does not own its lifetime. |
| `storage` | a `StorageBackend`. **Required.** Omitting it is an error, never a fallback — FR-014. |

`start()` subscribes to the encryption updates. `stop()` unsubscribes and flushes state.
Both are idempotent. The manager never installs a global handler and never mutates `client`.

## 2. The nine operations the first consumer needs

Named after the MCP tools they must serve, so a reader can see the mapping.

| Operation | Returns | Refuses when |
|---|---|---|
| `create(user)` | a chat in `requested` | the peer cannot accept secret chats; DH parameters fail §1.2 |
| `accept(chat_id)` | a chat in `ready` | the request expired; parameters fail §1.2 |
| `close(chat_id)` | nothing | already closed (states it, does not raise) |
| `list()` | every chat with its state, peer, TTL and fingerprint | — |
| `status(chat_id)` | one chat's detail, fingerprint included | unknown id |
| `set_ttl(chat_id, seconds)` | nothing | chat not `ready` |
| `send_message(chat_id, text, entities=None)` | the sent message's id | chat not `ready` or `rekeying` |
| `send_file(chat_id, path, ...)` | the sent message's id | as above; unreadable file |
| `read_history(chat_id, limit)` | delivered messages, in order | unknown id |
| `save_file(message, path)` | the written path | the file's key fingerprint does not match (§6) |

`send_message` and `send_file` resolve when **Telegram accepts the ciphertext**, not when
the peer acknowledges (spec Assumptions). Acknowledgement arrives as an event.

## 3. Events

Delivered to handlers the application registers. Every event names its chat.

| Event | Carries |
|---|---|
| `ChatRequested` | an incoming request awaiting `accept` |
| `ChatReady` | established; fingerprint included so the application can show it |
| `ChatClosed` | terminal, with the reason as a **shape** |
| `MessageReceived` | decrypted text or media reference, in conversation order |
| `MessageAcknowledged` | the peer's counter passed a message this side sent |
| `ServiceActionReceived` | which of the thirteen, and its parameters |
| `DecryptFailed` | the failure's shape — **never the ciphertext, never a partial plaintext** |

A failed decrypt is an event, not an exception thrown through the application's update loop
(spec Assumptions). Failures the protocol says must end the chat produce `ChatClosed` too.

## 4. Errors

One hierarchy, rooted at `SecretChatError`. Every one carries a shape and never a value —
Principle IV, and FR-015.

| Error | Raised when |
|---|---|
| `ParameterRejected` | §1.2 validation failed |
| `ChatNotReady` | an operation needs `ready` and the chat is not |
| `ChatClosed` | the chat is terminal |
| `StorageRequired` | constructed without a backend |
| `LayerUnsupported` | the peer cannot reach layer 73 (FR-017) |
| `ResendUnsatisfiable` | a span outside retention was requested (§3.7) |

`str(e)` on any of them contains: the error name, the chat id, and a bounded description.
It contains no key, no plaintext, no ciphertext and no protocol object repr. **A test drives
every one of these paths and asserts exactly that.**

## 5. StorageBackend

The interface in [../data-model.md](../data-model.md) §5. An application implements it or
uses one of the two shipped. `save` must be atomic across key, fingerprint, pending key and
counters — a partial write leaves a chat that decrypts nothing.

## 6. What is NOT exported

- Anything under `crypto`, `dh`, `framing`, `sequence`, `rekey` — the protocol internals.
  They are tested directly; they are not a supported surface.
- The generated TL schema.
- Any way to supply a key, skip a check, or lower the layer. There is no "trust me" flag,
  because the one thing this package sells is that the checks ran.
