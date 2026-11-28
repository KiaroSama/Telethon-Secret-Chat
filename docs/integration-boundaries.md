# Integration and upgrade boundaries

These contracts describe `v0.6.0`. Earlier release tags retain their original
behavior.

## Upgrading to `v0.6.0`

`Chat` now exposes `incoming_key_cutover`, `accepted_exchange`, and
`gap_key_fingerprints` so rekey authentication survives sequence gaps and
restarts. Update complete `Chat` struct literals and exhaustive patterns. Back
up your store before upgrading. If you must downgrade, restore the pre-upgrade
backup: older versions do not enforce the provenance recorded by `v0.6.0`.

A legacy snapshot with one key and buffered gaps remains readable. If a legacy
snapshot has two keys, buffered gaps, and no provenance identifying which key
verified each message, `v0.6.0` refuses to open it and leaves the file unchanged.

At startup, `StrongSecretChatManager` uses `DurableIngress` to retire
stale encrypted message rows for explicitly closed chats or absent chats with
no pending creation. Those rows no longer stop other chats from recovering.
Rows for `Requested` and `Waiting` chats stay pending until the chat is ready.
Replay preserves paused custody, including pauses that cannot resume, and
continues to process `UpdateEncryption` as before.

## Authorization ownership

`GrammersTransport` binds to the remotely verified user, home DC, a local opaque
tag of the permanent authorization key, and the caller's generation string.
The current authorization's session-management hash can be zero; it is not used
as the device's cryptographic identity. Each adapter operation checks local key
continuity before dispatch and before publishing its result. Multipart upload
and download steps repeat that guard. These checks cannot undo an RPC already
dispatched when an external authorization change occurs.

The binding domain changed from `kiaro-grammers-binding-v1` to
`kiaro-grammers-binding-v2`. Files created with the former derived identity fail
the new owner/AAD checks on ordinary opens; ordinary opens never migrate. For
an explicit offline rebind after quiescing and backing up store files, use
`binding_migration` (`migrate_chat_store_v1_to_v2`, `migrate_custody_store_v1_to_v2`,
`migrate_ingress_store_v1_to_v2`) with `BindingOwnershipProof`, which requires
both the exact old `v1` identity and the verified `v2` identity and rewrites one
file at a time (no cross-store transaction). Changing an owner string to bypass
the check does not prove identity. Custom transports retaining the same valid
binding can read existing records; the new accepted-batch field defaults to empty
for older writer output.

## Establishment and restart

The caller owns update routing, pending outgoing handshakes, and the returned
established chat. Request/accept futures hold their private exchange state in
memory. Dropping a future after dispatch may leave a remote request or acceptance
without the local exponent/key. Retain and resume a pinned future when a timeout
must not discard that state. Initial exchanges have no durable restart journal.

After establishment, persistent managers checkpoint keys, counters, rekey state,
gaps, outboxes, and owed receive batches. File stores persist across process
restart; `MemoryStorage` only persists within its object. Strong mode adds the
[explicit consumer settlement contract](strong-ingress.md).

## Text TTL and retry identity

Persistent and transient wrappers infer each new message's TTL from protocol
state at sequence reservation, under the final chat-state lock after pending
outbox transmission. Peer TTL updates processed while that send waits apply to
the new frame. Public RAM `SecretChatManager::send_text` keeps its explicit TTL
argument; an explicit retry must use the same text and TTL.

For newly staged persistent requests, `send_text_with_id` binds the caller's ID
to the text independently of the conversation TTL. While the outbox retains that
ID, retrying the same ID and text returns and, when needed, retransmits the exact
original ciphertext, even after a TTL change or restart. Changing the text is
refused. A successful peer acknowledgement removes the retained entry; the ID
is not a permanent application deduplication key after that retirement.

Existing outbox entries keep their original identity digest and ciphertext. A
legacy retry first checks the existing text/TTL digest with the current TTL. If
that no longer matches, it authenticates the original outgoing frame with a
still-held key, recovers its original TTL, and checks the original digest. This
does not change the stored format, relabel the identity, or encrypt a replacement
frame. If the original key has already retired and the current-TTL digest does
not match, the request is safely refused with `retry_pending` guidance. The old
digest alone cannot recover the missing TTL or distinguish changed text; no
retired key is restored or retained merely for this fallback. `retry_pending`
can still transmit entries marked pending using their exact retained ciphertext
without decrypting it. Entries whose RPC already succeeded remain retained for
peer acknowledgement and are not forcibly retransmitted; this call can correctly
return `0`. New implicit identities do not need that legacy recovery path.

The serialized record shape is unchanged, but the new implicit digest needs the
updated matcher. Downgrading to older code may reject `send_text_with_id` for a
new-format retained entry; `retry_pending` can still resend its original frame
when the entry is pending.
Unchanged serialization does not imply bidirectional retry compatibility.

## Media, saved history, and orchestration

`media` provides metadata, encryption/decryption, forwarding descriptors, voice
header detection, and file-saving helpers. `GrammersTransport` exposes encrypted
file RPC operations. `orchestration::MediaOrchestrator` layers bounded upload,
download, and zero-reupload forwarding over `EstablishmentTransport` while the
caller still owns protocol framing, sequence state, and retries. Use
`send_encrypted_media_with_frame` when the frame must include the freshly
generated file key and IV; the prebuilt-frame method treats frame bytes as
opaque. The local adapter's file-size ceiling is not a claim about every
Telegram account's server upload allowance. The file fingerprint checks key/IV
identity; it is not ciphertext authentication.

`AutoSaveStore` is independent opt-in storage. Applications may call
`AutoSaveStore::record_message` directly or use `orchestration::AutoSaveSubscription`
to route saved records per subscribed chat. Subscriptions store only records
explicitly passed to `on_message`; they install no protocol or event hook.
Persistent and transient wrappers do not retain a duplicate RAM event queue;
their receive batches remain available through their existing delivery APIs.
Direct RAM managers keep the public `drain_events` API. Unknown stored fields
and duplicate identities are refused instead of being silently discarded by a
later rewrite.

`orchestration::BoundedCustodySettlement` tracks caller-owned pending
deliveries in memory with ledger-wide ceilings, non-consuming polling, explicit
acknowledgement, and missed-ingress pause handling. It does not drain the
durable raw frame queue inside `BoundedSecretChatManager`.

## Archive publication

`export_saved_chat` renders saved plain text and media records in Desktop-style
HTML/JSON and copies referenced assets. `orchestration::export_rich_saved_chat`
extends that staged publication with formatted entities (`RichTextEntity`), replies,
and service events (`ServiceArchiveEvent`), writing both the base export and rich
output before a single atomic directory rename. The destination must be new or
empty; append and in-place regeneration are not supported.

The exporter stages all content in a private sibling directory, cleans it on
ordinary failure, and renames the completed directory into place. Existing
nonempty output is preserved. On Windows, a verified-empty destination is removed
immediately before the rename and recreation is attempted if publication fails; its uninterrupted
existence is not promised. Parent directories must be private. Directory rename
publishes completed contents but is not a power-loss durability guarantee, and
newly created parent directories may remain after failure.

## Evidence and sizing boundaries

Offline tests exercise real message crypto, authenticated store files, controlled
RPC outcomes, property-based state invariants, and restart/cancellation behavior.
They do not establish live Telegram interoperability, signed release availability,
or physical power-loss recovery. See [interoperability testing tiers](interop-testing.md)
for the opt-in manual live workflow and [checkpoint storage sizing](benchmarks/checkpoint-storage.md)
for whole-file snapshot scaling thresholds. Inspect checks and release assets at
the exact reviewed commit.
