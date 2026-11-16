# Encrypted commit-before-ack ingress

`ingress::DurableIngress` and `manager::StrongSecretChatManager` use the bundled
native grammers supplement. Ordinary post-SDK update delivery cannot establish
this guarantee. No second client, session import, login or device conversion is
performed. Use the crate's canonical SDK re-exports.

## Setup order

1. The application constructs its original `SenderPool` and `Client` from its
   existing authorization. Keep the runner unstarted and the update receiver
   unclaimed, with no invocation dispatched by any handle clone. A pool already
   running, with an earlier queued invocation, or attached to another owner is refused.
2. Derive `AuthorizationBinding::from_session_parts` from the known self user,
   home DC and authorization key ID; never persist the raw authorization key as
   the binding. Open `EncryptedIngressStorage` with a caller-owned 32-byte AEAD
   key, separate from Telegram authorization and secret-chat keys.
3. Construct `DurableIngress::new(storage, IngressLimits)` and call
   `prepare(&original_pool_handle, Some(verified_MessageBoxes_checkpoint)).await`
   for a fresh store. The baseline must represent the caller's verified existing
   update state. A reconstructed journal uses `None` and refuses replacement.
4. Start the original runner. `StrongSecretChatManager::new(original_client,
   ingress.clone(), CustodyLimits).await` proves the exact original pool with a
   CSPRNG ping cookie and verifies the server self-user response is a non-bot.
   Unverified attachment permits no acknowledgement.
5. Start the strong manager and create the original `UpdatesReceiver`. Feed its
   updates to the strong manager to wake journal replay. The manager consumes
   committed journal rows, not arbitrary post-dispatch ciphertext as receipts.
6. Stop/settle the strong manager before handing off storage or stopping the
   original pool. The application owns receiver/runner shutdown.

The exact `prepare` and binding signatures are in the Rust API. The supplement is
already included in source/Git installs; [SDK preparation](rust-migration.md) is
an optional hash-verification/auditing tool, not a runtime patch step.

Choose an owner-controlled physical directory for custody. The encrypted store
refuses symbolic links in the path or its ancestors, including temporary
directory aliases. Resolve the trusted base directory before appending the new
custody filename.

## Custody and acknowledgement

A complete live input is staged before any volatile SDK forwarding. The receiver
applies it to a cloned MessageBoxes, commits the full envelope, emitted updates
and next checkpoint, then replaces the observed machine. Checkpoints include
account/channel counters, date/seq, held gaps, deadlines and difference ownership.
Caller-issued account/channel difference responses are captured whole before the
RPC result is returned, including qts-less secret messages and slices.

Operational SDK callbacks run on one ordered blocking worker per original pool.
The pool owns that thread independently of its callers' Tokio runtimes. RPC
permit cleanup is queued on the same executor when its guard is dropped, so
destroying a short-lived runtime cannot strand an issued request or bypass
callback ordering.
The worker owns its serial permit until the synchronous callback actually ends,
including after cancellation of its async waiter. The receiver's checkpoint read,
state transition and commit share one operation, so a caller-issued difference
cannot replace the checkpoint between those steps. Initialization also stages
updates received alongside the connection result before another network step can
flush acknowledgements. Constructors, direct synchronous protocol/storage calls
and exclusive pre-start attachment remain synchronous interfaces.

Every original handle clone shares request guards. Explicit receivedQueue and
implicit difference acknowledgement cannot exceed committed qts. Known wrappers
are inspected recursively; unknown flags/wrappers and caller-packed MTProto
containers refuse. Unissued/cancelled permits cannot capture results. Too-long
differences, capacity/age/commit uncertainty, identity/DC replacement and stopped
owners fence participation. A saved frontier must agree with its checkpoint even
after completed batches are pruned. Existing protocol records without their
journal refuse a fresh baseline.

Protocol receive/establishment/terminal changes and their exact input application
cursor commit to the same encrypted State. Unrelated settings or failed decrypts
do not settle a journal row. Unknown terminal rows wait while held creations or
pending request identities could still claim them; healthy peer work can proceed.
No-create proof is only used after all relevant creation ownership is absent.

AES-256-GCM encrypts the whole state using a random nonce and binding/version AAD.
The file backend retains atomic replacement, exclusive ownership and refusal on
uncertain persistence. Protect and back up the caller's AEAD key separately.
The frontend/session projection exposes only a durably backed checkpoint.

## Evidence and limits

The offline tier uses the real original Client/receiver, modified SDK and a
synthetic deleting-server model for live, difference, explicit acknowledgement
and failed receipt commits. It also exercises filesystem faults, restart,
backpressure, wrong-client proof, cancellation and mixed streams. Fixtures use
burned keys; these are not real-account claims.

This policy provides finite addon content/task ownership, not total process-RSS,
external side-effect atomicity or erasure of SDK/OS copies. Application callbacks
must define their own durable side-effect contract. Cooperative cleanup can
refuse settlement; it cannot forcibly terminate arbitrary external blocking code.
See [bounded mode](bounded-mode.md), [architecture](architecture.md) and
[the exact inventory](../migration/README.md).
