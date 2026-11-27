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
the new owner/AAD checks. There is no automatic rebinding or migration. Keep the
original files and exact old binding available for an explicitly reviewed
migration; changing an owner string to bypass the check does not prove identity.
Custom transports retaining the same valid binding can read existing records;
the new accepted-batch field defaults to empty for older writer output.

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

## Media and saved history

`media` provides metadata, encryption/decryption, forwarding descriptors, voice
header detection, and file-saving helpers. `GrammersTransport` exposes encrypted
file RPC operations. Applications still compose them with message metadata,
update routing, progress/cancellation, and manager sequencing. No manager-level
media upload/download/forward workflow is supplied. The local adapter's file-size
ceiling is not a claim about every Telegram account's server upload allowance.
The file fingerprint checks key/IV identity; it is not ciphertext authentication.

`AutoSaveStore` is independent opt-in storage. The application calls
`start_auto_save`, supplies `SavedSecretMessage` records to `record_message`, and
owns media fetching. Enabling it does not attach an automatic manager event hook.
Unknown stored fields and duplicate identities are refused instead of being
silently discarded by a later rewrite.

## Archive publication

`export_saved_chat` renders saved plain text and media records in Desktop-style
HTML/JSON and copies referenced assets. `SavedSecretMessage` has no complete rich
entity or service-event model, so this is not full Desktop export feature parity.
The destination must be new or empty; append and in-place regeneration are not
supported.

The exporter stages all content in a private sibling directory, cleans it on
ordinary failure, and renames the completed directory into place. Existing
nonempty output is preserved. On Windows, a verified-empty destination is removed
immediately before the rename and recreation is attempted if publication fails; its uninterrupted
existence is not promised. Parent directories must be private. Directory rename
publishes completed contents but is not a power-loss durability guarantee, and
newly created parent directories may remain after failure.

## Evidence boundaries

Offline tests exercise real message crypto, authenticated store files, controlled
RPC outcomes, and restart/cancellation behavior. They do not establish live
Telegram interoperability, signed release availability, or physical power-loss
recovery. Inspect checks and release assets at the exact reviewed commit.
