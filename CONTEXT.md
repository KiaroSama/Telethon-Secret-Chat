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

## Messages

**Wrapper layer**:
The layer number written on one received message's wrapper; it may never go down from one
accepted message to the next.
_Avoid_: layer (unqualified), message layer

**Capability layer**:
The highest layer the peer has said it understands, raised by a layer notice or any higher
wrapper and never lowered; our outgoing layer is capped by it.
_Avoid_: peer layer, remote layer (in writing), layer (unqualified)

**Gap**:
A hole in the peer's message numbering: a message arrived before one it follows. Later
messages wait until the hole is filled.
_Avoid_: missing message, loss

**Resend**:
The request asking the peer to send the messages of one gap again, sent once per gap.
_Avoid_: retry (that is our own transmission again), re-request

**Retained outbox**:
The messages this side sent, kept with their exact encrypted bytes until the peer's counter
shows it processed them, so a resend can be answered with the original bytes.
_Avoid_: send queue, outbox (unqualified), sent history

**Delivery mailbox**:
Received messages accepted in order but not yet handed to the application; it survives a
restart, so a message is handed over at least once.
_Avoid_: inbox, receive queue, pending messages

**Acknowledgement**:
The peer's counter passing a message this side sent: proof the peer processed it, which is
later than Telegram accepting it.
_Avoid_: delivery receipt, read receipt, sent confirmation

**Media kind**:
One of the eight names a file is sent as (photo, video, document, audio, animation, sticker,
video note, voice note); it decides how the receiving client shows the file.
_Avoid_: media type, file type, mime type

**Media metadata**:
What a receiving client shows about a file before downloading it (a video's length and size,
a preview picture, a voice note's waveform, a sticker's emoji). The sender supplies it; the
package never reads it out of the file.
_Avoid_: attributes (in writing), file info, tags

**Media reference**:
Everything needed to save or forward one received file later, the file's one-time key
included, kept by the application after a restart if it chooses. Holding one means being able
to read that file.
_Avoid_: file id, media handle, file pointer

**Forward**:
Sending a file already on Telegram's servers into a chat again under its original one-time
key, without downloading or uploading it; it may go to a different chat than it came from.
_Avoid_: re-send (that is `send_file`), copy, share

**Tombstone**:
The record of a closed chat, kept with its keys erased so the chat id cannot be revived,
until the application forgets it.
_Avoid_: dead chat, closed record (in writing), deleted chat

**Auto-save**:
Keeping every message and file of every secret chat on this side as it happens, because the
server keeps none; the saved copy outlives message and chat deletes.
_Avoid_: archive (in writing), backup, export (that renders saved messages for reading)

**Delete (one side)**:
Ending a chat and removing everything this side holds of it - tombstone, history and queued
messages - while the peer keeps its own copy and only sees the chat end. Exporting first is
the application's job.
_Avoid_: forget (that removes only a tombstone), clear, wipe

**Delete for both sides**:
A delete that also asks Telegram to have the peer's client erase its history of the chat; best
effort, because a chat already ended on the server can no longer carry the request. A peer's
delete for both sides removes the chat from this side on arrival.
_Avoid_: revoke, flush (that clears history in a chat that stays open)

## Verification

**Interop tier**:
The tests that exchange real messages with an official Telegram client on a real
account; they never run in CI.
_Avoid_: live tests, integration tests, e2e tests

**Operator**:
The person driving the official client during an interop-tier run.
_Avoid_: user, tester, peer (the peer is the account, not the person)

**Pair tier**:
The tests where this package is on BOTH ends of a secret chat, between two of the
owner's accounts over the real Telegram server; no person takes part and they never run in
CI. They show the package survives the real server, not that it agrees with an official
client: both ends are the same code, so only the interop tier is interop evidence.
_Avoid_: interop (for these), two-account interop, self test
