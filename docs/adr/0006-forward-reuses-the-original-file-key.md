# 0006. A forward reuses the original file key

- Status: accepted
- Date: 2026-09-29

## Context

A file received in a secret chat is stored on Telegram's servers encrypted with a one-time key
that travels inside the message (§6.1). Sending it on can either download, decrypt, re-encrypt
under a new key and upload again, or point a new message at the same server-side file and
repeat its key. TDLib and the official clients do the second
(`DocumentsManager::get_secret_input_media`, tdlib/td `42e6a52`), and a live check on
2026-09-29 showed the server accepts the same file handle in another chat
(`docs/design/forward-file.md`).

## Decision

`forward_file` re-sends by handle: the new message carries the source's key, iv, size, mime
type, thumbnail and attributes, and the file handle is `inputEncryptedFile(id, access_hash)`.
Nothing is downloaded or uploaded. An application that wants a fresh key sends the file again
with `send_file`.

## Consequences

- A forward costs one small RPC whatever the file size, up to 2 GB.
- Everyone who held the original key (the first sender and every earlier recipient) can read
  the forwarded copy if they obtain the server-side file. This is stated in the docstring and
  the README, and a `MediaReference` is key material for exactly that one file.
- A source larger than 2000 MB still needs the peer at layer 143, as for `send_file`.
