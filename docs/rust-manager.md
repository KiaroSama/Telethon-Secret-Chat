# Rust managed secret chats

`manager::SecretChatManager<S, T>` adds handshake recovery, the retained send
queue, ordered delivery, callbacks, media, saved copies and lifecycle ownership
to `Protocol`. `S` is explicit storage and `T` is a `manager::Transport`. No
second Telegram client or session is created. The application owns the grammers
client, authorization, networking task and update stream.
The managed API accepts no shared-key override. The separate low-level
`protocol` module intentionally exposes validated establishment for protocol
integrations and synthetic fixtures; it is not a shortcut on a manager.

For the native transport, construct `GrammersTransport` from the application's
existing client and use `SecretChatManager::verified(transport, storage,
Options::default()).await`. This derives a binding from public, read-only RPCs on
that client: self user, current authorization and connected DC. `start` verifies
the persisted binding before recovery sends. A different authorization refuses
participation. The lower-level `new(transport, storage, digest, options)` is useful
for custom transports; native grammers operations still verify that supplied
digest before any Telegram operation. Custom transports implementing `Transport`
may return `None` from `authorization_digest` only when they explicitly accept
responsibility for binding their storage.

Call `start().await` before dispatching application updates. Pass each
`grammers_tl_types::enums::Update` from the same client's stream to
`handle_update(&update).await`; ordinary Telegram updates are ignored. The
manager does not install a global handler or alter the client's update routing.
Applications requiring commit-before-server-ack ingress must use the repository's
verified ingress seam; passing an already acknowledged ordinary update is not
strong ingress custody.

## Establishment and recovery

`create(InputUser).await` stores the DH secret and request random ID before
`requestEncryption`. A retry after a lost response reuses that request identity.
`accept(chat_id).await` stores the private exponent before `acceptEncryption` and
reuses it after an uncertain result. Duplicate requested/ready updates cannot
replace an established key. Early ready and discard updates are reconciled with
the initial request. Unknown waiting/ready updates outside an active creation
are discarded in durable mode as lost requests.

`list()` and `status(chat_id)` return detached `ChatSnapshot` values with state,
participant, layer, TTL, fingerprint and visualization hash. They contain no DH
private exponent or key bytes. `read_history(chat_id, limit)` returns a detached
bounded runtime cache of received messages; it is not a disk archive.

`send_message(chat_id, text, entities, reply_to).await` parses Markdown when
`entities` is `None`; pass `Some(Vec::new())` for literal text. A successful send
returns the original random ID. An uncertain send returns `Error::SendPending`
with the chat and random ID. `retry_pending(chat_id).await` reuses the exact
retained ciphertext and ID; it never invents a replacement message. The manager
also retries owed sends during startup and before a subsequent outgoing message.
Automatic rekey checks run before outgoing content, and `rekey(chat_id).await`
requests an explicit exchange. Protocol responses, resends and public sends share
one per-chat wire-effect lock. Before issuing a queued effect, the manager checks
that the chat remains sendable and that the pending retained identity and frame
still match; confirmed, removed or rewritten stale effects issue no new RPC.
Already issued RPCs can still complete after a local close.
An older stored chat with a usable key can exchange ordinary traffic without DH
metadata. If it needs to rekey, participation closes with a fixed missing-DH
reason instead of leaving a failed service at the head of its mailbox.

`set_ttl`, `mark_read`, `screenshot` and `set_typing` send the corresponding
service action. `delete_messages` and `flush_history` withdraw retained content
without dropping sequence slots or pending protocol actions. Incoming messages
and service actions drain in their original sequence order, including recovery:
an earlier message reaches saving/callback delivery before a later delete is
applied. Saved copies remain independent of protocol deletion.
`delete_messages` consumes an `IntoIterator<Item = i64>` once, including lazy
iterators.

## Handlers and shutdown

Register `on(event, static_handler_name, callback)` for synchronous callbacks
returning the crate's `Result<()>`. These run inline. Register `on_async` for an
owned future or `on_task` for an existing Tokio `JoinHandle<Result<()>>`. The
async callback factory runs during emission; its returned future is owned and
dropped on cancellation even if its first poll has not happened. Handler
names accept ASCII letters, digits and underscores. Events are `ChatRequested`,
`ChatReady`, `ChatClosed` (`ChatClosedEvent` is an alias), `MessageReceived`,
`MessageAcknowledged`, `ServiceActionReceived`, `DecryptFailed` and `SendFailed`.

A callback `Err` is isolated and logged with a fixed message and its registered
name; neither the error value nor message contents are logged. Async failures
and task termination are observed. Rust panics are programmer failures and may
invoke the application's installed panic hook; return `Err` for expected
recoverable failures. The library does not replace a process-global panic hook.

`stop().await` coordinates one shutdown independently of a canceled waiter. It
cancels and joins owned file downloads and handler tasks, then saves protocol
state and releases the storage owner. A handler may call `stop` and complete its tail;
its task is not canceled by its own shutdown. `start` and `stop` are idempotent.
A later `start` first completes a failed shutdown before recovery. Uploads from
an older run cannot mutate the new run, and the new run does not inherit their
outbound locks. `active_handler_tasks()` reports active owned handler tasks.
`settle().await` waits for the callbacks owned when it starts, excluding its own
callback task. Finite modes apply their operation and cleanup deadlines. Use
`on_async_with_cleanup` or `on_task_with_cleanup` for asynchronous cancellation
cleanup; dropping a Rust future itself cannot await cleanup.
Finite deadlines drop the owned transport future. Reservations remain charged
while an accepted worker or callback still owns the corresponding payload. An RPC already queued in grammers may still execute remotely;
retained identities and generation checks remain necessary. Custom transport
futures must yield, and their destructors must complete promptly.

`close(chat_id).await` ends participation according to storage mode.
`forget(chat_id).await` removes a closed durable tombstone.
`delete_secret_chat(chat_id).await` removes local durable protocol state, and
`delete_secret_chat_both_sides(chat_id).await` additionally requests remote
history deletion, returning whether that request succeeded. Unknown or repeated
removal is refused. Saved conversation folders require separate explicit removal.

## Files

`media::MEDIA_KINDS` defines photo, video, document, audio, animation, sticker,
video_note and voice_note. `SendFileOptions` carries caption, MIME override,
explicit kind, reply ID and `Metadata`. Photo sends retain the baseline's
image-document representation. Stickers and round videos refuse captions.
Metadata is validated before encryption/upload: permitted fields, integer
bounds, paired thumbnail/size, accepted JPEG or sticker WEBP signatures and
thumbnail dimensions, waveform length and media-specific attributes.

`send_file(chat_id, source, options).await` accepts `FileSource::path`, named
`FileSource::bytes`, or a named owned seekable `FileSource::stream`. Streams send
only bytes remaining from their initial position. Ogg previews read at most
64 KiB, handle partial reads, and restore the stream position. The bounded encrypting reader
uploads directly through the existing client without a plaintext temporary file.
The server's encrypted-file handle is retained for retransmission.

`MessageReceived.media_reference` is present for a usable attached encrypted
file. `MediaReference::to_json` and `from_json` support explicit serialization;
this data contains the file key and IV and must be protected like key material.
`Debug` deliberately omits them. Parsing validates the TL constructor family,
complete consumption, lengths and fingerprint. `save_file(reference, path)` and
`save_message_file(message, path)` validate the fingerprint before any download
or destination creation. Native downloads use temporary ciphertext and atomically
publish the validated plaintext file.

`forward_file(chat_id, reference, caption, reply_to)` and
`forward_message_file(chat_id, message, caption, reply_to)` reuse the original
server file, key and IV without download or upload. The target layer determines
the document constructor. Files exceeding the legacy signed size limit require
layer 143 or newer.

`receive_file(reference).await` and `receive_message_file(message).await` return
verified plaintext in capped RAM without creating a file. For an explicit
plaintext limit, use `receive_file_with_limit(reference, maximum).await`; an
oversized advertised file refuses before starting the download.

## Saved copies and custody modes

FileStorage defaults to `saved-secret-chats` beside the state file. Memory
storage has no default archive; `Options.auto_save_folder` supplies an explicit
one. `start_auto_save_secret_chats(folder).await` enables saving,
`stop_auto_save_secret_chats().await` settles current downloads and persists OFF,
and `auto_save_secret_chats()` returns the active destination. Explicit OFF
survives restart, including OFF set before the first public `start`. Relative
paths are anchored when installed.

`read_saved_messages(chat_id)` returns merged UTF-8 JSONL records;
`delete_saved_messages(chat_id)` explicitly removes that saved chat directory.
Messages include direction, date, text, entities, reply, TTL and media metadata.
Incoming files download independently; canceled downloads remain in a protected
pending-reference file for recovery. Completed rows point to plaintext saved
files, and their media metadata omits key/IV/reference. Expected save/download
failures log fixed text and do not prevent delivery or sending. This archive is
plaintext and needs filesystem protection and its own retention policy.
`export_saved_messages` reads the archive even after removal of the protocol
conversation and delegates to the native Desktop exporter. The destination
must not overlap the archive source.

Protected mode refuses save/export/delete/forget/flush/auto-save and suspends
locally while preserving keys. Bounded and ingress close stop local participation
without remote discard or custody deletion. Ingress auto-save is disabled.
Bounded and ingress callbacks do not acknowledge consumer obligations:
`pending_relay_deliveries(chat_id)` returns `custody::RelayDelivery` values with
original wire sequence IDs, and `acknowledge_delivery(chat_id, wire_sequence)`
releases an obligation only after the consumer's durable work. Wrong parity and
negative IDs are refused. Startup may redeliver an unacknowledged obligation;
consumers must deduplicate by chat and wire sequence.

Finite modes share one operation and callback task budget, operation deadlines,
and held-payload accounting. Nested manager calls reuse their task slot and
reserve their additional payload. A synchronous `Options.admission` callback
can refuse a public operation before its effect; refusing incoming protocol
input marks a continuity loss that cannot be resumed. Protected callbacks must
be owned async functions; missing consumers, failed callbacks and exhausted
budgets suspend participation while preserving key custody.
An owned expiry timer suspends aged volatile queues and held payloads even while
the application is idle, cancels their callbacks, and is joined before shutdown
releases storage. Expired payload reservations remain charged until their owners
drop them.
`TransientSecretChatManager::set_accepting_requests(false)` blocks new create,
accept and request intake while existing conversations continue.
`is_suspended(chat_id)` reports local refusal without exposing keys.

## Strong ingress manager

Prepare `DurableIngress` on the caller's original grammers sender and receiver
before it can receive live updates. `StrongSecretChatManager::new` takes that
same client, the prepared ingress and explicit custody limits. Construction
verifies the original session and claims one protocol writer; plain and volatile
storage cannot satisfy its Rust constructor type. `new_with_options` and
`with_transport_and_options` also accept `Options.admission` and callback cleanup
configuration without changing the verified client or enabling automatic saving.
The facade provides message/file operations, bounded RAM downloads, explicit
file saving, content delete/flush, and owned future/task callbacks without
exposing its internal protocol writer.

`handle_update` only wakes journal draining. The durable journal remains the
source of encrypted updates. Replay commits each message or establishment with
its matching cursor, keeps an unacknowledged consumer receipt through restart,
and continues healthy peers while another peer is paused. Returning `Ok` from
an unrelated operation does not settle a journal row. Unknown discarded updates
remain held while a creation owns a pre-RPC slot or a protected pending request,
including after cancellation; healthy later rows can continue. Strong shutdown fences
new acknowledgments, cancels and joins owned replay, then releases storage.
Calling shutdown from the replay task itself refuses instead of joining itself.
Automatic saving is disabled for this mode.

## Verification scope

The manager/media suites use a synthetic local Telegram server and temporary
storage only. They check managed two-party handshakes, retained identities,
callbacks, generation races, real FileStorage reopen, eight media kinds,
metadata refusal before upload, forwarding without byte movement and saved
copies. They do not log in or perform live Telegram operations. The migration
ledger records compared, executed legacy equivalents individually; passing these
suites does not establish parity for every legacy case. CI separately requires
the repository's complete 1851-case migration gate.

Native file transfers run synchronous source reads, AES-IGE refills, RAM file
decryption, and final filesystem publication on Tokio's blocking pool. Cancellation fences final
publication, and manager shutdown retains and joins accepted workers. A caller's
synchronous `Read` implementation must eventually return: Rust cannot forcibly
interrupt an arbitrary blocking read or destructor. If that prevents bounded
cleanup, shutdown reports failure and retains the worker owner; startup refuses
new I/O until those workers actually finish. An RPC already queued in grammers
may still execute after its awaiting future is canceled.

Callback admission and registration share a shutdown barrier. Already returned
callback tasks keep their task permit, payload reservation and registry owner
until the actual task finishes, including when its original Tokio runtime is
lost. If a running callback cannot be canceled before the cleanup deadline,
shutdown reports failure and keeps the protocol writer owned; a later stop can
finish the join. Cancelable cleanup after a child finishes remains bounded by
its cleanup deadline. Diagnostic task callbacks after shutdown still have their
returned handles retrieved, canceled and observed. A canceled borrowed
upload retains the caller's original reader and its current cursor; reads already
performed are not undone. A synchronous rewind waits for all accepted refills,
including queued workers, before resetting the cursor for another attempt.
