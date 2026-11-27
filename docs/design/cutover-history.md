# Design note: history and media references after the TDLib cutover

Design spike, 2026-09-29. Output: the evidence, the two designs, what a prototype of
the media-reference seam showed, and the recommendation. No code shipped.

## 1. Evidence

**The consumer already owns durable history.** telegram-mcp removed TDLib on 2026-09-21
(`71cd071 feat: remove tdlib`, merged as PR #46) and keeps its own two-way history in
`telegram_mcp/secret_history.py` (read-only check, file at `dba84b1`, 2026-09-26). Its module
docstring states the decision: the package's in-memory list is "the right scope for a
library", and the application keeps history because TDLib's backend "kept a durable local
database of BOTH directions". It records outgoing messages itself (`record_sent`, called after
a delivered send) and incoming ones from `MessageReceived` (`record_received`), keeps at most
500 messages per chat, writes the file owner-readable, and removes entries on
`clear_secret_history` / `delete_secret_message`. The published shape is TDLib's
(`message_id`, `is_outgoing`, `date`, `type` such as `messageVoiceNote`, `text` or
`caption`, `self_destructs_after_seconds`, `file_id`), with `message_id` = the `random_id`.

**Media after a restart is deliberately lost there.** `save_secret_media`
(`telegram_mcp/tools/secret_messaging.py`, `_live_message`) looks the message up in the
package's IN-MEMORY `read_history(chat_id, 10_000)` because the durable record holds no key
("writing it to disk would turn a convenience file into a second place an encrypted
conversation can be read from"). After a restart the tool answers that the file can no longer
be decrypted.

**What saving needs.** `files.receive` reads exactly: `chat_id`; from the media, `key` (32
bytes), `iv` (32 bytes), `size`; from the attached `EncryptedFile`, `id`, `access_hash`,
`dc_id` and `key_fingerprint` (checked against `md5(key + iv)` folded, §6.2, before a byte is
written). Nothing else of `MessageReceived` is needed.

**Consequence of the bounded history for the consumer.** With the history bounded at `history_limit`
(default 1000), `_live_message`'s `read_history(chat_id, 10_000)` can only find the most
recent 1000 received messages per chat. The consumer should pass `history_limit` to match
what it expects, or move to option A's reference.

## 2. The two designs

### Option A - the application owns history (recommended)

- A serializable `MediaReference` (frozen dataclass): `chat_id, file_id, access_hash, dc_id,
  size, key, iv, fingerprint, mime_type, attributes` (attributes as plain data, not TL
  objects), with `to_dict()` / `from_dict()` and a `media_reference` field on
  `MessageReceived` (`None` without media).
- `save_file(message_or_reference, path)` accepts either; the fingerprint check is unchanged.
- Optionally a `MessageSent(chat_id, random_id, seq_no, text, media_reference)` event, emitted
  after a send is accepted by Telegram, so an application does not have to rebuild what it
  sent. The consumer does not need it today (it records sends itself).
- `read_history`, `flush_history`, `DeleteMessages` and restart behave as today; nothing new
  is stored by the package. The reference carries the one-time file key, so where it is kept
  is the application's decision, documented as such (Principle IV: it never appears in a
  repr, a log or an error).

### Option B - a package-owned `HistoryStore`

- An ABC beside `StorageBackend`, an in-memory default and a JSON/SQLite implementation,
  holding both directions, refilled at `start()`, reached by `flush_history` and
  `DeleteMessages`.
- Consequences: decrypted plaintext at rest inside the package, and TTL enforcement then
  belongs here too (ADR-level reversal of the settled "stored and transmitted, not enforced");
  every delete path must reach the store; the consumer's existing history would be duplicated.

## 3. Prototype

A throwaway script (outside the repository, deleted) built a reference from what a
`MessageReceived` holds, serialized it to JSON (290 bytes), rebuilt it, and fed it to the
unchanged `files.receive` with a fake client serving the ciphertext: the decrypted file was
byte-identical to the original. `files.receive` already duck-types its input, so the seam
needs a data type and `save_file` accepting it, not a new decryption path.

## 4. Recommendation and open questions

**Option A.** The consumer already keeps durable two-way history by its own decision, the
package keeps no plaintext at rest beyond the delivery mailbox, and the reference is the one
missing piece: with it, an application that chooses to persist the key can save media after a
restart, and one that does not loses nothing it has today.

Owner decision needed: whether the consumer should persist `MediaReference` (a file key at
rest beside its history) or keep today's "save while running" rule.

Open question (unchanged from the audit): `DecryptedMessage.ttl` is per message in the schema,
but the package always sends the chat's TTL. Whether official clients honour a per-message
value is UNVERIFIED (§5). Experiment: send two messages in one chat with different `ttl`
values through a temporary local change and record what the official client's timers show.

**Follow-up feature acceptance criteria (option A):**

- `MessageReceived.media_reference` is set exactly when the message carries a file, and
  `MediaReference.from_dict(ref.to_dict()) == ref`.
- `save_file(reference, path)` writes the same bytes as `save_file(message, path)` and refuses
  a reference whose fingerprint does not match its key and iv, before writing.
- No repr, log line or error contains the key or iv (leak test extended).
- README documents that a stored reference is key material for that one file.
