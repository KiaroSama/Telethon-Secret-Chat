# Design note: forwarding an encrypted file without re-uploading it

Design spike, 2026-09-29. Output: the oracle's behaviour, the API, the security
statement and the acceptance criteria. The live check is UNVERIFIED (it needs the operator).

## Today

`send_file` always encrypts and uploads: a "forward" of a received file costs download,
decrypt, re-encrypt and upload (up to 2 GB since layer 143). Yet the pieces for a forward
exist: after a file send `outbox._transmit` stores `InputEncryptedFile(id, access_hash)` in the
retained record, a received message carries the `EncryptedFile` (`MessageReceived.file`) and
the one-time key in its media (`MessageReceived.media`), and `manager._send(chat, message,
file=...)` sends any file handle with `messages.sendEncryptedFile`. Only building an
`InputEncryptedFile` for a NEW message is missing (§6.4, item 4).

## The oracle

TDLib reuses the server-side file and its original one-time key. At tdlib/td commit
`42e6a5259551178d1dab54a22ad96d14bd906e20` (2026-09-25):

- `DocumentsManager::get_secret_input_media` (`td/telegram/DocumentsManager.cpp`, from line
  652): when the file already has a remote location, the handle becomes
  `main_remote_location->as_input_encrypted_file()`, and the media is built from the same
  `file_view`.
- `FullRemoteFileLocation::as_input_encrypted_file` (`td/telegram/files/FileLocation.h`, line
  486): `make_tl_object<telegram_api::inputEncryptedFile>(common().id_, common().access_hash_)`.
- `SecretInputMedia::SecretInputMedia` (`td/telegram/SecretInputMedia.cpp`, from line 16):
  `decryptedMessageMediaDocument` is filled with `encryption_key.key_slice()` and
  `encryption_key.iv_slice()` of that same file - the ORIGINAL key and iv; the thumbnail,
  mime type, size and attributes are copied, the caption is the new message's.
- `SecretChatActor::send_message_impl` (`td/telegram/SecretChatActor.cpp`, line 240) takes the
  `InputEncryptedFile` as given and records it in the outbound log event; line 1403 sends it
  with `messages.sendEncryptedFile`.

So a forward is a new encrypted message whose media carries the original key/iv/size and
whose file handle is `inputEncryptedFile(id, access_hash)`. The fingerprint the peer checks is
unchanged because key and iv are unchanged.

## Live check (UNVERIFIED)

Whether a handle received in chat A may be sent into chat B (server-side ownership of `id` /
`access_hash`) is not documented. The check, for the operator: with two throwaway chats to the
second account, receive a file in chat A, send a message in chat B whose media copies A's
key/iv/size and whose handle is `inputEncryptedFile(A.file.id, A.file.access_hash)` - through a
temporary local change, reverted afterwards - and record whether it opens on the official
client. Close both chats at the end. If the server refuses a cross-chat handle, the API
becomes same-chat only.

## API for the follow-up feature

```python
async def forward_file(chat_id, source, *, caption="", reply_to=None) -> int  # random_id
```

- `source` is a `MessageReceived` with media, or a `MediaReference` (see
  [cutover-history.md](cutover-history.md)).
- The new `DecryptedMessage` carries a `decryptedMessageMediaDocument` whose `key`, `iv`,
  `size`, `mime_type`, `thumb*` and `attributes` are copied from the source; `caption` and
  `reply_to_random_id` are the new message's. The file handle is `InputEncryptedFile(id,
  access_hash)`; nothing is uploaded or downloaded.
- The retained record stores that `InputEncryptedFile` from the start, so a §3.7 resend needs
  no handle rewrite (today's rewrite after upload does not apply).
- Layer: a source larger than 2000 MB needs the peer at layer 143 or above, as for
  `send_file` (TDLib drops the handle below it, `SecretInputMedia.cpp`).

## Security statement (for the docstring and README)

A forwarded copy is encrypted with the ORIGINAL one-time file key, so anyone who held that
key - the original sender and every earlier recipient - can decrypt the forwarded copy if
they obtain the server-side file. This is inherent to §6.1's design and is what official
clients do; an application that needs a fresh key must re-send with `send_file`.

## Acceptance criteria

- Unit: the sent media reuses key, iv and size byte for byte; the handle is
  `InputEncryptedFile(id, access_hash)`; no upload or download call is made; the retained
  record holds the handle; a resend replays it unchanged.
- Unit: a source without media is refused before anything is sent.
- Live: a forwarded file opens on the official client (same chat at least; cross-chat if the
  live check above allows it).
