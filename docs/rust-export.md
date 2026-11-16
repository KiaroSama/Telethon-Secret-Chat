# Native Rust Desktop-compatible export

The `export` module implements the Telegram Desktop v7.2.10 chat-export model, HTML and JSON renderers, file naming, media admission, thumbnails and the single-chat request flow. All library, tool and test execution is Rust. Generated HTML retains Desktop's static CSS, JavaScript and 42 PNG assets as output data. No Python, Qt or C image runtime is needed.

The migration ledger maps all 683 original exporter and saved-export scenarios to individual Rust cases. Independent fixtures retain complete expected byte output and filesystem effects from the unchanged original scenarios; the Rust dispatchers invoke the actual model and rendering functions. Stateful writer, request, retry and archive-preservation checks run directly against native implementations.

## Export a saved secret chat

Use `manager.export_saved_messages(chat_id, destination, &self_user, &peer_user, &settings)` after autosave has been enabled. The archive remains available after autosave is stopped and after its protocol chat is removed. The call reads saved copies without reconnecting to Telegram. Protected storage refuses an ordinary saved-copy export.

For archive readers outside the manager:

```rust,no_run
use kiaro_secret_chat::export::{export_saved_records, Format, Settings, User};
use std::path::Path;

# fn example(records: &[serde_json::Value]) -> Result<(), Box<dyn std::error::Error>> {
let me = User { id: 1, is_self: true, first_name: "Me".into(), ..Default::default() };
let peer = User { id: 2, first_name: "Peer".into(), ..Default::default() };
let settings = Settings { format: Format::Both, time_zone: Some("UTC".into()), ..Default::default() };
let result = export_saved_records(records, Path::new("export"), &me, &peer,
    &settings, Some(Path::new("saved source")))?;
assert!(!result.takeout);
# Ok(()) }
```

Participants need positive IDs. Date bounds are nonnegative Unix seconds; `single_peer_from` is inclusive and nonzero `single_peer_till` is exclusive. Media flags default to photos with an 8 MiB limit and accept at most 4000 MiB. JSON configuration can be validated through `saved::SavedExportOptions::try_from(&serde_json::Value)`; boolean sizes and string-valued media lists are rejected. Validation and resolved source/destination overlap checks happen before writes.

Saved random IDs become dense Desktop IDs in archive order. Replies map to retained IDs. TTL changes, screenshots and history-flush services have Desktop equivalents; deletion services have no corresponding visible Desktop event. The owner's saved copies are retained without enforcing self-destruct timers. Offline custom emoji use the unavailable placeholder. Exports copy admitted media and never remove archive messages or files. `ExportResult.messages` counts the submitted chat rows; writers apply date filtering to rendered output. Error displays exclude message bodies, source paths and raw filesystem diagnostics. Rust-owned output handles close when the writer is dropped, including copy failures.

## Export a regular Telegram chat through Grammers

`grammers_client::GrammersExportClient::new(client.clone())` uses the existing caller-owned `grammers_client::Client`. It creates no client, authorization or session and does not own the application's update runner. Set `Settings.single_peer` to an encoded peer ID and `single_peer_input` to its constructor-tagged input peer, including its access hash. Pass the adapter, settings, an `ExportWriter` and `Environment` to `fetch::export_single_chat`.

The driver requests the self user and countries list, opens a takeout with the Desktop flags and size ceiling, and falls back to ordinary requests on a takeout refusal. It walks history splits in order, migrates old basic-group IDs, hydrates partial rich messages, resolves custom emoji, downloads media in 128 KiB chunks and retries expired file references at most three times. RPC requests and downloads use the same supplied client. Country phone formatting belongs to each export, avoiding process-wide state shared across simultaneous exports. The injected `fetch_client::ExportClient` interface supports deterministic offline request fixtures and custom transports.

## Image compatibility and provenance

The thumbnail path reproduces Qt 5.15.19 smooth scaling, matrix/TRC ICC transforms, libjpeg's integer IDCT, chroma upsampling and color conversion. JPEG encoding uses the pure Rust `mozjpeg-rs` revision pinned in `Cargo.toml`; trellis settings, scan costs and baseline output match the original byte oracles, including the Adobe RGB JPEG fixture. The safe private decoder adapts `image-rs/jpeg-decoder` 0.3.2; its complete MIT and Apache licenses remain in `src/export/jpeg_decoder/`. `NOTICE` retains Telegram Desktop, Qt and Imlib2 attribution.

`tests/fixtures/tdexport-source-rust.json` distinguishes hashes of rewritten/adapted native files from the unchanged **emitted asset bytes**. Native source hashes are project integrity records, not a claim that Rust files are unchanged upstream Python or C++ files. Source-bundle packaging includes `src/**`, embedded assets and legal notices. This project currently distributes its patched SDK graph from Git/source bundles; it does not claim a crates.io publication.

Official API references checked during the migration: [`account.initTakeoutSession`](https://core.telegram.org/method/account.initTakeoutSession) defines the shared `files`/`file_max_size` flag, and [`upload.getFile`](https://core.telegram.org/method/upload.getFile) defines download offsets, chunk limits and file-reference errors. The adapter also preserves canonical TL acronym spellings when converting official Rust types into export data.
