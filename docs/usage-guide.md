# Use Kiaro Secret Chat with grammers

Start with an authorized grammers user client that your application already runs.
The same client sends secret-chat requests and receives their updates. Test with
an account and a peer you control before enabling the flow for other users.

## Prerequisites

- Git and Rust 1.95 or newer. The project develops on Rust 1.100.0. On Windows,
  install the Visual Studio C++ build tools and Windows SDK for MSVC. On Linux,
  install a native linker such as GCC.
- A grammers `Client`, its sender-pool runner, and the application's update
  receiver. Secret chats are for user accounts.
- A private directory for state, downloaded files, or exports. Ordinary file
  storage contains secret-chat keys. Set owner-only ACLs on Windows and keep
  the directory out of shared or synchronized storage.

Add the crate to your application's `Cargo.toml`:

```toml
[dependencies]
kiaro-secret-chat = { git = "https://github.com/KiaroSama/Telethon-Secret-Chat", branch = "main" }
```

Keep your application's `Cargo.lock`. To move the dependency only when you
review a change, use `rev` with a full commit SHA in place of `branch`.

Import `grammers_client`, `grammers_mtsender`, `grammers_session`, and
`grammers_tl_types` through `kiaro_secret_chat` wherever your integration needs
those crates. This keeps the client, pool, session, and raw TL types in one Cargo
dependency graph. A client from a separately resolved `grammers-client` can
have an incompatible type even if its version number matches. The bundled SDK
needs no consumer `[patch]`. See [SDK compatibility](rust-migration.md#one-sdk-graph).

## 1. Attach the manager

Pass a clone of your already authorized client. Keep the original sender-pool
runner running so the manager's verification RPCs can complete. Register the
callbacks your app needs before `start`, which can recover pending work.

```rust
use kiaro_secret_chat::{
    error::Result,
    grammers_client::Client,
    manager::{Options, SecretChatManager},
    storage::FileStorage,
    transport::GrammersTransport,
};
use std::path::Path;

type Manager = SecretChatManager<FileStorage, GrammersTransport>;

async fn attach(client: &Client, private_dir: &Path) -> Result<Manager> {
    SecretChatManager::verified(
        GrammersTransport::new(client.clone()),
        FileStorage::open(private_dir.join("secret-chats.json"))?,
        Options::default(),
    )
    .await
}
```

In the next examples, `manager` is the value returned by `attach`. Register
handlers, call `manager.start().await?`, then send each raw update through the
manager. On shutdown, stop dispatching new updates and call `manager.stop().await?`
before shutting down your pool. The manager releases its store and settles work
it owns; the application still owns the runner.

For a disposable test, pass `MemoryStorage::default()` in place of
`FileStorage::open(...)` and use `SecretChatManager<MemoryStorage, GrammersTransport>`.
The store cannot recover a chat after the process exits.

## 2. Create or accept a chat

Give `create` the `grammers_tl_types::enums::InputUser` for the peer, resolved
through your existing client. A numeric user ID alone may not provide the access
hash needed to construct an input user.

```rust
use kiaro_secret_chat::{chat::ChatSnapshot, error::Result, grammers_tl_types::enums::InputUser};

async fn request_chat(manager: &Manager, peer: InputUser) -> Result<ChatSnapshot> {
    manager.create(peer).await
}
```

On receipt, pass the raw TL update to `manager.handle_update(&update).await`.
If your receiver yields a high-level grammers update instead, pass `update.raw()`:

```rust
use kiaro_secret_chat::grammers_client::update::Update;

async fn dispatch(manager: &Manager, update: &Update) {
    manager.handle_update(update.raw()).await;
}
```

The manager handles raw `Encryption` and `NewEncryptedMessage` updates and
ignores unrelated updates. Keep dispatching while a chat is pending. A
`ChatRequested` callback carries `chat_id` and `peer_user_id`; show the requester
and call `manager.accept(chat_id).await?` only after your user approves. Wait for
`ChatReady` or check `manager.status(chat_id)?.state` for `ChatState::Ready`
before sending. Save the secret-chat ID separately from the peer's user ID.

## 3. Send text and process replies

Register a `MessageReceived` callback before you start the manager. This example
hands each message to an application callback without writing it to the log.

```rust
use kiaro_secret_chat::{error::Result, manager::{Event, MessageReceived}};

fn on_message(
    manager: &Manager,
    deliver: impl Fn(MessageReceived) -> Result<()> + Send + Sync + 'static,
) -> Result<()> {
    manager.on("MessageReceived", "app_message", move |event| {
        if let Event::MessageReceived(message) = event {
            deliver(*message)?;
        }
        Ok(())
    })
}
```

For long-running application work, use `on_async` and return errors instead of
panicking. Receive errors such as malformed frames use `DecryptFailed` events.
Keep logs limited to fixed diagnostic messages; message bodies, raw updates,
and media references may contain private material.

```rust
use kiaro_secret_chat::error::Result;

async fn reply(manager: &Manager, chat_id: i32, incoming_id: i64) -> Result<i64> {
    manager.mark_read(chat_id, vec![incoming_id]).await?;
    manager.send_message(chat_id, "I received it.", None, Some(incoming_id)).await
}
```

The third argument is the entity list. `None` parses Markdown;
`Some(Vec::new())` sends literal text. The fourth argument is the original
message's `random_id` when replying, or `None` for a new message. Keep the ID
returned by `send_message`; later acknowledgements arrive as
`MessageAcknowledged`. If the send returns `Error::SendPending`, use
`retry_pending(chat_id).await` to reuse the recorded ciphertext and random ID.
Sending the text again would create a different message.

- `set_typing(chat_id, None).await` sends the default typing action.
- `set_ttl(chat_id, 60).await` sends a 60-second timer for future messages. Use
  `0` to turn it off. The timer does not erase plaintext archives or exports.
- `delete_messages(chat_id, [random_id]).await` sends a deletion service action
  and withdraws retained content. Saved copies are separate.
- The secret-chat manager has no edit-in-place method. Send a correction as a
  new message, or explicitly delete the original and send the replacement.

## 4. Transfer encrypted media

Use the manager to encrypt and send a file with the same client. In this example
`path` is a file the user selected and `chat_id` is a ready secret chat.

```rust
use kiaro_secret_chat::{error::Result, manager::SendFileOptions, media::FileSource};
use std::path::Path;

async fn send_attachment(manager: &Manager, chat_id: i32, path: &Path) -> Result<i64> {
    let options = SendFileOptions {
        caption: "Meeting notes".into(),
        ..Default::default()
    };
    manager.send_file(chat_id, FileSource::path(path), options).await
}
```

`FileSource::bytes` accepts named bytes; `FileSource::stream` accepts a named,
owned seekable stream. The manager checks media kind, metadata and caption rules
before upload. For example, stickers and round videos refuse captions. See
[media options](rust-manager.md#files) for supported kinds and limits.

When a received message has a usable attachment, `media_reference` contains
its encrypted server-file reference. For an ordinary manager, download and
save with `save_message_file` or use `receive_message_file` to download into
bounded memory. The secret-chat manager has no method named `download_media`;
`media::download_file` is the lower-level operation for an existing reference.

```rust
use kiaro_secret_chat::{error::Result, manager::MessageReceived};
use std::path::{Path, PathBuf};

async fn save_attachment(
    manager: &Manager,
    message: &MessageReceived,
    destination: &Path,
) -> Result<PathBuf> {
    manager.save_message_file(message, destination).await
}
```

To forward without downloading or uploading the payload again, call
`forward_message_file(target_chat_id, &message, "", None).await`. If you already
have a `MediaReference`, use `forward_file` instead. Forwarding reuses the
existing encrypted server file. Both calls send a new secret-chat message to
the target chat. Ask for the user's consent before forwarding. Treat serialized
references like keys; they contain the file key and IV.

Ordinary `FileStorage` defaults to a `saved-secret-chats` archive beside the
state file. `start_auto_save_secret_chats(&folder).await` chooses an explicit
folder and `stop_auto_save_secret_chats().await` persists OFF. The saved files
and message records are plaintext; disabling auto-save does not erase them.

## 5. Compare fingerprints and rotate keys

`manager.status(chat_id)?` returns a snapshot with a numeric `key_fingerprint`
and, when available, the original key's `key_hash`. Compare the visual hash
with your peer over a separate trusted channel. Render `KeyVisualization.rows`
using the four entries in `PALETTE`, or compare its `hex` text. A match confirms
what both devices show; a mismatch means you should stop sending and investigate.

```rust
use kiaro_secret_chat::{error::Result, visualization::{KeyVisualization, key_visualization}};

fn key_picture(manager: &Manager, chat_id: i32) -> Result<Option<KeyVisualization>> {
    let snapshot = manager.status(chat_id)?;
    snapshot.key_hash.as_deref().map(key_visualization).transpose()
}
```

The manager checks automatic PFS triggers during outgoing work. You can start a
manual exchange with `manager.rekey(chat_id).await?`; keep processing updates so
the exchange can complete. `exchange_in_progress` tracks the pending exchange in
the snapshot. The original verification picture stays stable while the current
fingerprint changes. See the [rekey guide](rust-rekey.md) for the old-key window.

## 6. Pick a storage and custody mode

| Mode | Use it when |
| --- | --- |
| `MemoryStorage` | A test or disposable process can lose its chats on exit. |
| `FileStorage` | A single process needs state and send recovery across restarts. Protect the directory and backups. |
| `BoundedSecretChatManager` | You need finite durable protocol custody. Wrap a backend in `custody::CustodyStorage` with `CustodyLimits`; handle pause status and resume explicitly. |
| `TransientSecretChatManager` | You need bounded transient payloads. Wrap `ProtectedFileStorage` in `TransientStorage` with `TransientLimits`; ordinary auto-save and saved export are unavailable. |
| `StrongSecretChatManager` | You need encrypted ingress committed before the SDK can acknowledge the covered updates. It uses `DurableIngress` with `EncryptedIngressStorage`. |

The bounded and transient types are manager aliases over their respective
storage wrappers. Pass an explicit store and use the binding for the same
client. After a refusal, do not assume every missed update can be recovered.
See [bounded mode](bounded-mode.md) and [transient mode](transient-mode.md)
for what is retained, what can be resumed, and which operations refuse.

For strong ingress, start from a pool that has not run or dispatched an
invocation. Prepare `DurableIngress` on that original pool with the verified
MessageBoxes baseline for a fresh store. Start the runner, attach and start
`StrongSecretChatManager`, then feed updates from the original receiver to wake
journal replay. Follow the [strong ingress setup order](strong-ingress.md#setup-order)
for the binding, encryption key, and restart rules. Wrapping updates after
ordinary SDK dispatch cannot provide commit-before-ack custody.

## 7. Export saved messages

Enable auto-save before the messages you intend to keep. Supply the two actual
participants as `export::User` records with positive IDs, marking your own
record `is_self`. Choose `export::Format::Html`, `Json`, or `Both` in
`export::Settings`.

```rust
use kiaro_secret_chat::export::{self, ExportResult, Format, Settings, User};
use std::path::Path;

fn export_chat(
    manager: &Manager,
    chat_id: i32,
    destination: &Path,
    me: &User,
    peer: &User,
) -> export::Result<ExportResult> {
    let settings = Settings { format: Format::Both, ..Default::default() };
    manager.export_saved_messages(chat_id, destination, me, peer, &settings)
}
```

Use a private destination separate from the saved archive. The export reads
local records without contacting Telegram and does not delete the original.
The [export guide](rust-export.md#export-a-saved-secret-chat) describes filters,
media admission, and the relationship between saved copies and TTL.

## 8. Verify downloads and run local tools

For a publisher-signed native package, verify its detached archive signature
with the pinned public key, then verify the archive and internal checksums.
On Windows, also check the tools' Authenticode signer and timestamp. Follow
[Publisher signatures](SIGNING.md) for these checks and the separate GPG route
for the original `v0.3.0` release.

From a source checkout, run whichever tool you need:

```sh
cargo run --locked --bin migration-audit -- --require-complete
cargo run --locked --bin matrix-evidence -- records
cargo run --locked --bin prepare-sdk -- --source /path/to/official/grammers --output /path/to/new/output
cargo run --locked --bin generate-schema -- --output /path/to/generated.rs
```

- `migration-audit` checks the 1,851 legacy mappings.
- `matrix-evidence` expects the seven CI evidence directories under `records`,
  each with its `rust-tests.txt`. A log from one local run cannot pass. The gate
  checks 1,855 identities: 1,851 mapped legacy cases and four native AES-IGE cases.
- `prepare-sdk` verifies the pinned upstream checkout before it writes a new
  directory. Git consumers already have the bundled SDK and can skip this step.
- `generate-schema` uses the bundled schema and `rustfmt` by default. Add
  `--check` to verify a previously generated output without rewriting it.

See [native Rust verification](rust-ci.md) for matrix legs and evidence policy.
