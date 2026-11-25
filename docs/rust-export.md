# Native Rust Desktop-compatible export

The `export` module implements Telegram Desktop chat-export models, HTML and JSON renderers, file naming, media admission, thumbnails and the single-chat request flow. Its independent migration fixtures retain the v7.2.10 baseline; relevant export changes through v7.4.4 are applied explicitly. All library, tool and test execution is Rust. Generated HTML retains Desktop's static CSS, JavaScript and 42 PNG assets as output data. No Python, Qt or C image runtime is needed.

The migration ledger maps all 683 original exporter and saved-export scenarios to individual Rust cases. Independent fixtures retain complete baseline byte output and filesystem effects from the original scenarios. Explicit overlays identify intentional upstream updates and native corrections without replacing those baseline records. The Rust dispatchers invoke the actual model and rendering functions. Stateful writer, request, retry and archive-preservation checks run directly against native implementations.

## Export a saved secret chat

Use `manager.export_saved_messages(chat_id, destination, &self_user, &peer_user, &settings)` after autosave has been enabled. The archive remains available after autosave is stopped and after its protocol chat is removed. The call reads saved copies without reconnecting to Telegram. Protected storage refuses an ordinary saved-copy export.

For archive readers outside the manager:

```rust,no_run
use kiaro_secret_chat::export::{Format, Settings, User, export_saved_records};
use std::path::Path;

fn example(records: &[serde_json::Value]) -> Result<(), Box<dyn std::error::Error>> {
    let me = User {
        id: 1,
        is_self: true,
        first_name: "Me".into(),
        ..Default::default()
    };
    let peer = User {
        id: 2,
        first_name: "Peer".into(),
        ..Default::default()
    };
    let settings = Settings {
        format: Format::Both,
        time_zone: Some("UTC".into()),
        ..Default::default()
    };
    let result = export_saved_records(
        records,
        Path::new("export"),
        &me,
        &peer,
        &settings,
        Some(Path::new("saved source")),
    )?;
    assert!(!result.takeout);
    Ok(())
}
```

Participants need positive IDs. Date bounds are nonnegative Unix seconds; `single_peer_from` is inclusive and nonzero `single_peer_till` is exclusive. Media flags default to photos with an 8 MiB limit and accept at most 4000 MiB. JSON configuration can be validated through `saved::SavedExportOptions::try_from(&serde_json::Value)`; boolean sizes and string-valued media lists are rejected. Validation and resolved source/destination overlap checks happen before writes.

Saved random IDs become dense Desktop IDs in archive order. Replies map to retained IDs, including targets on later HTML pages. TTL changes, screenshots and history-flush services have Desktop equivalents; deletion services have no corresponding visible Desktop event. Positive TTLs without a named day/week duration retain their numeric seconds. The owner's saved copies are retained without enforcing self-destruct timers. Offline custom emoji use the unavailable placeholder. Exports copy admitted media and never remove archive messages or files. `ExportResult.messages` counts rows within the selected date range, matching dialog metadata and rendered output. Error displays exclude message bodies, source paths and raw filesystem diagnostics. Saved-export failures call `ExportWriter::abort()` to close native output handles while caller-owned writers remain alive; dropping a writer also releases its resources. Custom writers with their own resources should implement `abort()`.

## Export a regular Telegram chat through Grammers

`grammers_client::GrammersExportClient::new(client.clone())` uses the existing caller-owned `grammers_client::Client`. It creates no client, authorization or session and does not own the application's update runner. Set `Settings.single_peer` to an encoded peer ID and `single_peer_input` to its constructor-tagged input peer, including its access hash. Pass the adapter, settings, an `ExportWriter` and `Environment` to `fetch::export_single_chat`.

The driver requests the self user and countries list, opens a takeout with the Desktop flags and size ceiling, and falls back to ordinary requests on a takeout refusal. It walks history splits in order, uses bounded-history counts and date pivots, resolves old basic-group migrations from full metadata or the migration service message, hydrates partial rich messages, resolves custom emoji, downloads media in 128 KiB chunks and retries expired file references at most three times. RPC requests and downloads use the same supplied client. Country phone formatting belongs to each export, avoiding process-wide state shared across simultaneous exports. The injected `fetch_client::ExportClient` interface supports deterministic offline request fixtures and custom transports.

## Image compatibility and provenance

The thumbnail path reproduces Qt 5.15.19 smooth scaling, matrix/TRC ICC transforms, libjpeg's integer IDCT, chroma upsampling and color conversion. JPEG encoding uses the pure Rust `mozjpeg-rs` revision pinned in `Cargo.toml`; trellis settings, scan costs and baseline output match the original byte oracles, including the Adobe RGB JPEG fixture. The safe private decoder adapts `image-rs/jpeg-decoder` 0.3.2; its complete MIT and Apache licenses remain in `src/export/jpeg_decoder/`. `NOTICE` retains Telegram Desktop, Qt and Imlib2 attribution.

`tests/fixtures/tdexport-source-rust.json` distinguishes hashes of rewritten/adapted native files from the **emitted asset bytes**. Native source hashes are project integrity records, not a claim that Rust files are unchanged upstream Python or C++ files. Source-bundle packaging includes `src/**`, embedded assets and legal notices. This project currently distributes its patched SDK graph from Git/source bundles; it does not claim a crates.io publication.

## Current Desktop reference

The latest stable release reviewed on 2026-11-16 is [v7.4.4](https://github.com/telegramdesktop/tdesktop/releases/tag/v7.4.4), commit [`bf4f714036f21c750d535dab199c811b3f9bfdd4`](https://github.com/telegramdesktop/tdesktop/commit/bf4f714036f21c750d535dab199c811b3f9bfdd4). The export-specific comparison starts at the original [v7.2.10](https://github.com/telegramdesktop/tdesktop/tree/d81a5ac270fdcb5315b49f813614caa71f85697f/Telegram/SourceFiles/export) reference. It covers changed export data types, request flow, HTML/JSON output and CSS; removal of Desktop's example-data builder needs no native runtime change.

Applied updates include valid JSON control and Unicode-separator escapes, finite JSON coordinates, bounded geographic points, escaped service fields and custom-emoji attributes, safe button URL schemes, TON address entities/rich text, Gram transfers, TON Connect requests and free-message-link services. The pinned SDK uses API Layer 234. For incoming Gram top-ups, JSON and HTML include a nonblank provider name. Bot invitations identify the inviter. Telegram Desktop omits on-chain gift offers from service action output, and this exporter does the same. File sizes already use deterministic English formatting; native image decoding already refuses SVG/SVGZ and the current assertions preserve that refusal.

The current CSS adds Desktop's `rich_ton_address` selector. Its official LF source is Git blob `90506ecf5b4babb8fa91184130e062dadb872500` (SHA256 `c043288908ea12b18cbe427c2fbae9daa1e58c889950e2655606fb5267395dbe`). The established emitted CRLF representation has SHA256 `1d70127586462ce7996aad2749e7994aca9dee615c3d8d38e2acd4d2a00ca611`. The other 43 emitted assets, including JavaScript and all 42 PNGs, retain their baseline hashes. Both sets and the current source reference are recorded in `tdexport-source-rust.json`; no fixture is represented as a new upstream capture when it remains a baseline oracle.

Official API references checked during the migration: [`account.initTakeoutSession`](https://core.telegram.org/method/account.initTakeoutSession) defines the shared `files`/`file_max_size` flag, and [`upload.getFile`](https://core.telegram.org/method/upload.getFile) defines download offsets, chunk limits and file-reference errors. The adapter also preserves canonical TL acronym spellings when converting official Rust types into export data.

## Runtime corrections and local SDK behavior

The runtime review adds explicit project corrections alongside the Desktop reference: unsorted saved archives use their complete date range, media limits also inspect the real source size, migrated replies keep their local identity, peer namespaces and split counts remain associated, empty JSON chat sections finish correctly, and errors restore the caller's JSON nesting. Bot permission messages choose the app or domain sentence from the corresponding field. Thumbnail selection rejects nonpositive or out-of-range TL dimensions before comparing areas. Slideshow selector buttons have distinct accessible names. Fixture overlays identify these project corrections separately from upstream updates and retain the original results.

Thumbnail input reads stop at 64 MiB, independently of the bounds on progressive coefficients, lossless planes and scaling allocations. Lossless restart/point-transform, channel interleaving and precision handling are tested with independent synthetic JPEG frames. Unsupported high-precision multicomponent frames are refused. ICC metadata reassembles complete numbered chunks, including short final chunks; RGB thumbnails retain only RGB profiles.

The distributed Grammers source graph also carries local parser and wrapper corrections. HTML/Markdown generators escape literal text at original UTF-16 positions, malformed mentions stay ordinary links, code delimiters preserve content and paragraph whitespace, and unrepresentable formatted dates are omitted. CommonMark emphasis follows Unicode punctuation flanking and excludes boundary whitespace while preserving the original text at unrepresentable boundaries; adjacent compatible inline-code, bold and italic ranges are merged. Successful sends without a mapped response body retain empty or optional message snapshots instead of encouraging a duplicate retry. Missing optional media IDs return `Option<i64>`. `Participant` exposes `peer: Peer` and `user() -> Option<&User>` so banned channel identities remain available. `Dialog::peer_ref()` returns `Option<PeerRef>`; pagination can use a cached session reference or return an error. `Message::refetch()` returns `Result<Option<Message>, InvocationError>` and preserves the original snapshot. Inline callbacks require an explicit response destination, and unavailable callback messages return an error. These API corrections are part of this project's source distribution; the original upstream reference hashes remain separate from the patched-source hashes.
