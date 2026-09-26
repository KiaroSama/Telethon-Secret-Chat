# Telethon Secret Chat

Telegram's end-to-end encrypted "secret chats" (MTProto 2.0) for Telethon: two devices
agree a key the server never sees, and every message between them is sealed with it.

## Keys

**Shared key**:
The 256-byte secret only the two devices of one secret chat hold; every message is
encrypted with it.
_Avoid_: auth key, chat key, session key

**Key fingerprint**:
The 64-bit number derived from a shared key that both ends compare to confirm they
derived the same key. No official client shows it to a person.
_Avoid_: key id, fingerprint (unqualified)

**Key visualization**:
The 36-byte value an official client draws as the key picture a person compares on
two screens. It comes from the chat's FIRST shared key and never changes afterwards.
_Avoid_: key image, key_hash (as a spoken name), fingerprint

**File key fingerprint**:
The 32-bit number that ties a file's one-time key to the message that carries it. It
proves the key belongs to that file, not that the file's bytes are intact.
_Avoid_: file fingerprint, checksum

## Rekeying

**Rekey**:
Replacing a chat's shared key with a fresh one while the conversation continues, so a
key stolen later cannot open earlier messages.
_Avoid_: key rotation, re-handshake, PFS (as a noun for the act)

**Initiator**:
The side that starts a rekey; it commits the new key after the other side accepts.
_Avoid_: requester (when rekeying), side A

**Previous key**:
The shared key a rekey replaced, kept only until nothing sent under it can still arrive,
then discarded.
_Avoid_: old key (in writing), backup key

## Verification

**Interop tier**:
The tests that exchange real messages with an official Telegram client on a real
account; they never run in CI.
_Avoid_: live tests, integration tests, e2e tests

**Operator**:
The person driving the official client during an interop-tier run.
_Avoid_: user, tester, peer (the peer is the account, not the person)
