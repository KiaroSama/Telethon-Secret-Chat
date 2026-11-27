# Integration and upgrade boundaries

These contracts describe the current source, including the unreleased corrections
after `v0.4.0`. The immutable release tag retains its original behavior.

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
to route saved records per subscribed chat. Unknown stored fields and duplicate
identities are refused instead of being silently discarded by a later rewrite.

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
