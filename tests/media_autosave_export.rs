#![forbid(unsafe_code)]

use std::{fs, io::Cursor, path::PathBuf};

use kiaro_secret_chat::{
    autosave::{AutoSaveStore, SavedMediaRecord, SavedSecretMessage},
    export_archive::{ExportFormat, ExportMediaFilter, ExportOptions, export_saved_chat},
    media::{
        EncryptedMediaKey, ForwardMediaReference, MediaMetadata, SecretMediaKind,
        build_forward_media_descriptor, compute_file_key_fingerprint, decrypt_media_payload,
        decrypt_media_stream, detect_voice_note, encrypt_media_payload, encrypt_media_stream,
        md5_digest, save_media_atomic,
    },
};
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn round_trips_all_media_kinds_and_padding_boundaries() {
    use SecretMediaKind::*;
    for (kind, name, size) in [
        (Photo, "photo", 0),
        (Video, "video", 1),
        (Document, "document", 15),
        (Audio, "audio", 16),
        (VoiceNote, "voice_note", 17),
        (VideoNote, "video_note", 31),
        (Sticker, "sticker", 32),
        (Gif, "gif", 65_553),
    ] {
        assert_eq!(kind.as_str(), name);
        assert_eq!(name.parse::<SecretMediaKind>().unwrap(), kind);
        assert_eq!(kind.allows_caption(), !matches!(kind, VideoNote | Sticker));
        let bytes = vec![0xa5; size];
        let (encrypted, key) = encrypt_media_payload(&bytes).unwrap();
        assert_eq!(encrypted.len(), size.div_ceil(16) * 16);
        assert_eq!(
            decrypt_media_payload(&encrypted, size, &key).unwrap(),
            bytes
        );
    }
    assert_eq!("animation".parse::<SecretMediaKind>().unwrap(), Gif);
    assert!("unknown".parse::<SecretMediaKind>().is_err());
}

#[test]
fn md5_matches_rfc_1321_vectors_and_file_fingerprint_is_little_endian() {
    for (input, expected) in [
        (b"".as_slice(), "d41d8cd98f00b204e9800998ecf8427e"),
        (b"abc".as_slice(), "900150983cd24fb0d6963f7d28e17f72"),
        (
            b"message digest".as_slice(),
            "f96b697d7cb7938d525a2f31aaf161d0",
        ),
        (
            b"12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
    ] {
        assert_eq!(hex::encode(md5_digest(input)), expected);
    }
    let key = std::array::from_fn(|i| i as u8);
    let iv = std::array::from_fn(|i| (i + 32) as u8);
    assert_eq!(compute_file_key_fingerprint(&key, &iv), -217_561_997);
}

#[test]
fn decrypts_pinned_grammers_aes_ige_vector() {
    let bytes = std::array::from_fn(|i| i as u8);
    let key = EncryptedMediaKey {
        key: bytes,
        iv: bytes,
        fingerprint: compute_file_key_fingerprint(&bytes, &bytes),
    };
    let ciphertext =
        hex::decode("e28112a53e5c89c7b1ea8071c133699fd4e86b26c4bac9fc5af1ab8ce27a44a4").unwrap();
    assert_eq!(decrypt_media_payload(&ciphertext, 32, &key).unwrap(), bytes);
}

#[test]
fn rejects_wrong_fingerprint_key_and_ciphertext_size() {
    let plain = [42; 33];
    let (bytes, key) = encrypt_media_payload(&plain).unwrap();
    assert!(decrypt_media_payload(&bytes[..bytes.len() - 1], plain.len(), &key).is_err());
    assert!(decrypt_media_payload(&bytes[..32], plain.len(), &key).is_err());
    assert!(decrypt_media_payload(&bytes, usize::MAX, &key).is_err());
    assert!(decrypt_media_payload(&bytes, 32, &key).is_err());
    let mut wrong = key.clone();
    wrong.fingerprint ^= 1;
    assert!(decrypt_media_payload(&bytes, plain.len(), &wrong).is_err());
    let mut wrong = key;
    wrong.key[0] ^= 1;
    assert!(decrypt_media_payload(&bytes, plain.len(), &wrong).is_err());
}

#[test]
fn carries_ige_state_across_stream_chunks() {
    let bytes: Vec<_> = (0..160_003).map(|i| (i % 251) as u8).collect();
    let mut ciphertext = Vec::new();
    let key = encrypt_media_stream(
        &mut Cursor::new(&bytes),
        &mut ciphertext,
        bytes.len() as u64,
    )
    .unwrap();
    let mut independent = ciphertext.clone();
    kiaro_secret_chat::grammers_crypto::aes::ige_decrypt(&mut independent, &key.key, &key.iv);
    assert_eq!(&independent[..bytes.len()], bytes);
    let mut restored = Vec::new();
    decrypt_media_stream(
        &mut Cursor::new(&ciphertext),
        &mut restored,
        ciphertext.len() as u64,
        bytes.len() as u64,
        &key,
    )
    .unwrap();
    assert_eq!(restored, bytes);
    assert!(encrypt_media_stream(&mut Cursor::new(b"short"), &mut Vec::new(), 16).is_err());
}

#[test]
fn forwarding_reuses_the_exact_remote_file_and_key_material() {
    let (_, key) = encrypt_media_payload(b"existing remote media").unwrap();
    let mut original = ForwardMediaReference {
        kind: SecretMediaKind::Photo,
        metadata: MediaMetadata::default(),
        file_id: 101,
        access_hash: -202,
        dc_id: 4,
        size_bytes: 21,
        key,
    };
    let forwarded = build_forward_media_descriptor(&original, Some("Replacement".into())).unwrap();
    assert_eq!(forwarded.file_id, 101);
    assert_eq!(forwarded.access_hash, -202);
    assert_eq!(forwarded.dc_id, 4);
    assert_eq!(forwarded.key.key, original.key.key);
    assert_eq!(forwarded.key.iv, original.key.iv);
    assert_eq!(forwarded.key.fingerprint, original.key.fingerprint);
    assert_eq!(forwarded.caption.as_deref(), Some("Replacement"));
    for kind in [SecretMediaKind::VideoNote, SecretMediaKind::Sticker] {
        original.kind = kind;
        assert!(build_forward_media_descriptor(&original, Some("not allowed".into())).is_err());
        assert!(build_forward_media_descriptor(&original, None).is_ok());
        assert!(
            build_forward_media_descriptor(&original, Some(String::new()))
                .unwrap()
                .caption
                .is_none()
        );
    }
    original.key.fingerprint ^= 1;
    assert!(build_forward_media_descriptor(&original, None).is_err());
}

fn ogg_page(flags: u8, packet: &[u8]) -> Vec<u8> {
    let mut page = vec![0; 27];
    page[..4].copy_from_slice(b"OggS");
    page[5] = flags;
    page[26] = 1;
    page.push(u8::try_from(packet.len()).unwrap());
    page.extend(packet);
    page
}

#[test]
fn recognizes_ogg_opus_voice_but_not_music_or_truncated_headers() {
    let mut header = *b"OpusHead\x01\x01\x00\x00\x80\xbb\x00\x00\x00\x00\x00";
    let mut voice = ogg_page(2, &header);
    voice.extend(ogg_page(0, b"OpusTags\0\0\0\0\0\0\0\0"));
    assert!(detect_voice_note(&voice, "Audio/Ogg; codecs=opus"));
    assert!(!detect_voice_note(&voice, "audio/mpeg"));
    assert!(!detect_voice_note(&voice[..voice.len() - 1], "audio/ogg"));
    let mut music = ogg_page(2, &header);
    music.extend(ogg_page(0, b"OpusTags\0\0\0\0\x01\0\0\0\x07\0\0\0TITLE=x"));
    assert!(!detect_voice_note(&music, "audio/ogg"));
    header[..8].copy_from_slice(b"\x01vorbis\0");
    let mut vorbis = ogg_page(2, &header);
    vorbis.extend(ogg_page(0, b"\x03vorbis\0\0\0\0\0\0\0\0"));
    assert!(!detect_voice_note(&vorbis, "audio/ogg"));
}

#[test]
fn atomic_download_never_overwrites_and_rejects_unsafe_names() {
    let directory = tempdir().unwrap();
    let first = save_media_atomic(directory.path(), "تصویر.bin", b"first", 6).unwrap();
    let second = save_media_atomic(directory.path(), "تصویر.bin", b"second", 6).unwrap();
    assert_eq!(second.file_name().unwrap(), "تصویر (1).bin");
    assert_eq!(fs::read(first).unwrap(), b"first");
    assert_eq!(fs::read(second).unwrap(), b"second");
    for name in [
        "../escape.bin",
        "./bad",
        "/abs",
        "C:bad",
        "a\\b",
        "..",
        "bad\0",
        "NUL.bin",
    ] {
        assert!(
            save_media_atomic(directory.path(), name, b"x", 1).is_err(),
            "{name:?}"
        );
    }
    assert!(save_media_atomic(directory.path(), "oversized.bin", b"longer", 2).is_err());
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

fn message(id: i64) -> SavedSecretMessage {
    SavedSecretMessage {
        id,
        chat_id: -7,
        sender_id: 8,
        sender_name: "Name <b>& فارسی".into(),
        outgoing: false,
        date_unix: 1_700_000_000 + id * 60 * 60 * 24,
        text: format!("Saved message {id}"),
        reply_to_msg_id: None,
        ttl_seconds: None,
        media: None,
    }
}

#[test]
fn autosave_reopens_encrypted_deduplicated_history_and_stops_without_deleting() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("saved.enc");
    let key = [42; 32];
    assert!(AutoSaveStore::open(PathBuf::from("not-explicit.enc"), key).is_err());
    let mut store = AutoSaveStore::open(path.clone(), key).unwrap();
    assert!(!store.record_message(message(1)).unwrap());
    store.start_auto_save(-7).unwrap();
    store.start_auto_save(6).unwrap();
    assert!(store.record_message(message(1)).unwrap());
    assert!(!store.record_message(message(1)).unwrap());
    let mut other_chat = message(1);
    other_chat.chat_id = 6;
    assert!(store.record_message(other_chat).unwrap());
    assert!(store.record_message(message(2)).unwrap());
    assert!(AutoSaveStore::open(path.clone(), key).is_err());
    drop(store);
    let disk = fs::read(&path).unwrap();
    assert!(
        !disk
            .windows(b"Saved message".len())
            .any(|s| s == b"Saved message")
    );
    assert!(AutoSaveStore::open(path.clone(), [43; 32]).is_err());
    let mut store = AutoSaveStore::open(path.clone(), key).unwrap();
    assert_eq!(store.list_saved_chats(), [-7, 6]);
    assert!(store.is_auto_save_enabled(-7));
    assert_eq!(store.read_saved_messages(-7, 1), [message(2)]);
    assert!(store.read_saved_messages(-7, 0).is_empty());
    store.stop_auto_save(-7).unwrap();
    assert!(!store.record_message(message(3)).unwrap());
    drop(store);
    let mut store = AutoSaveStore::open(path, key).unwrap();
    assert!(!store.is_auto_save_enabled(-7));
    assert_eq!(store.read_saved_messages(-7, 10), [message(2), message(1)]);
    assert_eq!(
        store.delete_saved_messages(-7, Some(&[1, 1, 555])).unwrap(),
        1
    );
    assert_eq!(store.read_saved_messages(-7, 10), [message(2)]);
    assert_eq!(store.delete_saved_messages(-7, None).unwrap(), 1);
    assert_eq!(store.list_saved_chats(), [6]);
}

fn attachment(file: &str, size: u64) -> SavedMediaRecord {
    SavedMediaRecord {
        kind: SecretMediaKind::Document,
        mime_type: "application/octet-stream".into(),
        file_name: file.into(),
        size_bytes: size,
        local_rel_path: Some(file.into()),
        width: None,
        height: None,
        duration_secs: None,
    }
}

fn export_options(output_dir: PathBuf) -> ExportOptions {
    ExportOptions {
        chat_id: -7,
        chat_title: "Private <chat>".into(),
        peer_id: 8,
        format: ExportFormat::HtmlAndJson,
        output_dir,
        assets_dir: None,
        media_root: None,
        media_filter: ExportMediaFilter {
            photos: true,
            videos: true,
            voice_messages: true,
            video_messages: true,
            stickers: true,
            gifs: true,
            files: true,
            size_limit_bytes: 8,
        },
        date_from_unix: Some(message(1).date_unix),
        date_to_unix: Some(message(5).date_unix),
        messages_per_page: 2,
    }
}

#[test]
fn exports_paginated_html_json_assets_and_only_permitted_media() {
    let directory = tempdir().unwrap();
    let media_root = directory.path().join("media");
    fs::create_dir(&media_root).unwrap();
    fs::write(media_root.join("small file.bin"), b"small").unwrap();
    fs::write(media_root.join("oversized.bin"), b"longer than the limit").unwrap();
    let mut messages: Vec<_> = (0..7).map(message).collect();
    messages.push(SavedSecretMessage {
        chat_id: 11,
        ..message(2)
    });
    messages[1].text =
        "<script>alert(1)</script> https://example.org/?q=a&b=c javascript:alert(2)".into();
    messages[2].media = Some(attachment("small file.bin", 5));
    messages[3].media = Some(attachment("oversized.bin", 5));
    messages[3].reply_to_msg_id = Some(1);
    messages[5].reply_to_msg_id = Some(0);
    messages.reverse();
    let mut options = export_options(directory.path().join("archive"));
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/tdexport");
    options.assets_dir = Some(assets.clone());
    options.media_root = Some(media_root);
    let report = export_saved_chat(&messages, &options).unwrap();
    assert_eq!(
        (
            report.exported_messages,
            report.copied_media,
            report.skipped_media
        ),
        (5, 1, 1)
    );
    assert_eq!(
        report.html_pages,
        ["messages.html", "messages2.html", "messages3.html"]
            .map(|name| options.output_dir.join(name))
    );
    let first_page = fs::read_to_string(&report.html_pages[0]).unwrap();
    assert!(first_page.contains("Private &lt;chat&gt;"));
    assert!(first_page.contains("Name &lt;b&gt;&amp; فارسی"));
    assert!(first_page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!first_page.contains("<script>alert(1)</script>"));
    assert!(first_page.contains("href=\"https://example.org/?q=a&amp;b=c\""));
    assert!(!first_page.contains("href=\"javascript:"));
    assert!(first_page.contains("href=\"messages2.html\""));
    let second_page = fs::read_to_string(&report.html_pages[1]).unwrap();
    assert!(second_page.contains("href=\"messages.html#message1\""));
    assert!(second_page.contains("href=\"messages3.html\""));
    let last_page = fs::read_to_string(&report.html_pages[2]).unwrap();
    assert!(!last_page.contains("href=\"messages.html#message0\""));
    let json: Value =
        serde_json::from_slice(&fs::read(report.json_path.unwrap()).unwrap()).unwrap();
    assert_eq!(json["name"], "Private <chat>");
    assert_eq!(json["type"], "personal_chat");
    assert_eq!(json["id"], 8);
    let items = json["messages"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[0]["id"], 1);
    assert_eq!(items[0]["from_id"], "user8");
    assert_eq!(items[2]["reply_to_message_id"], 1);
    let file = items[1]["file"].as_str().unwrap();
    assert!(file.starts_with("files/"));
    assert_eq!(fs::read(options.output_dir.join(file)).unwrap(), b"small");
    assert!(!options.output_dir.join("files/oversized.bin").exists());
    for asset in ["css/style.css", "js/script.js", "images/section_photos.png"] {
        assert_eq!(
            fs::read(options.output_dir.join(asset)).unwrap(),
            fs::read(assets.join(asset)).unwrap()
        );
    }
}

#[test]
fn export_rejects_parent_paths_before_creating_the_archive() {
    let directory = tempdir().unwrap();
    let parent = export_options(directory.path().join("new/../escape"));
    assert!(export_saved_chat(&[], &parent).is_err());
    assert!(!directory.path().join("escape").exists());
    let mut record = message(1);
    let mut options = export_options(directory.path().join("archive"));
    options.media_root = Some(directory.path().to_owned());
    for path in [
        "../escape.bin",
        "images/../escape.bin",
        "C:/escape.bin",
        "..\\escape.bin",
    ] {
        record.media = Some(attachment(path, 1));
        assert!(export_saved_chat(&[record.clone()], &options).is_err());
        assert!(!options.output_dir.exists());
    }
}

#[test]
fn empty_html_export_has_an_explicit_state_and_fallback_assets() {
    let directory = tempdir().unwrap();
    let mut options = export_options(directory.path().join("archive"));
    options.format = ExportFormat::Html;
    let report = export_saved_chat(&[], &options).unwrap();
    assert!(report.json_path.is_none());
    assert_eq!(report.html_pages.len(), 1);
    assert!(
        fs::read_to_string(&report.html_pages[0])
            .unwrap()
            .contains("No saved messages")
    );
    assert!(options.output_dir.join("css/style.css").is_file());
    assert!(options.output_dir.join("js/script.js").is_file());
}
