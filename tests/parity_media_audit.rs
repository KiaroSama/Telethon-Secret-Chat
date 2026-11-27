#![forbid(unsafe_code)]

use std::{
    fs,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use kiaro_secret_chat::{
    autosave::{SavedMediaRecord, SavedSecretMessage},
    export_archive::{ExportFormat, ExportMediaFilter, ExportOptions, export_saved_chat},
    media::{
        EncryptedMediaKey, ForwardMediaReference, MediaMetadata, SecretMediaKind,
        build_forward_media_descriptor, compute_file_key_fingerprint, decrypt_media_stream,
        detect_voice_note, encrypt_media_stream,
    },
};
use tempfile::tempdir;

fn options(output_dir: PathBuf) -> ExportOptions {
    ExportOptions {
        chat_id: 7,
        chat_title: "Archive audit".into(),
        peer_id: 9,
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
            size_limit_bytes: 1024,
        },
        date_from_unix: None,
        date_to_unix: None,
        messages_per_page: 1,
    }
}

fn message(id: i64, relative: &str) -> SavedSecretMessage {
    SavedSecretMessage {
        id,
        chat_id: 7,
        sender_id: 9,
        sender_name: "Saved sender".into(),
        outgoing: false,
        date_unix: 1_700_000_000 + id,
        text: format!("Saved message {id}"),
        reply_to_msg_id: None,
        ttl_seconds: None,
        media: Some(SavedMediaRecord {
            kind: SecretMediaKind::Document,
            mime_type: "application/octet-stream".into(),
            file_name: relative.into(),
            size_bytes: 5,
            local_rel_path: Some(relative.into()),
            width: None,
            height: None,
            duration_secs: None,
        }),
    }
}

fn entries(path: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|item| item.unwrap().path())
        .collect();
    entries.sort();
    entries
}

#[test]
fn archive_asset_failure_does_not_publish_or_block_a_retry() {
    for preexisting in [false, true] {
        let directory = tempdir().unwrap();
        let output = directory.path().join("archive");
        if preexisting {
            fs::create_dir(&output).unwrap();
        }
        let assets = directory.path().join("assets");
        fs::create_dir_all(assets.join("css")).unwrap();
        let stylesheet = assets.join("css/style.css");
        fs::write(&stylesheet, b"body{}").unwrap();
        let permissions = fs::metadata(&stylesheet).unwrap().permissions();
        let mut readonly = permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&stylesheet, readonly).unwrap();
        let mut settings = options(output.clone());
        settings.assets_dir = Some(assets.clone());
        assert!(export_saved_chat(&[], &settings).is_err());
        fs::set_permissions(&stylesheet, permissions).unwrap();
        if preexisting {
            assert!(output.is_dir());
            assert!(entries(&output).is_empty());
        } else {
            assert!(!output.exists());
        }
        assert_eq!(
            entries(directory.path()),
            if preexisting {
                vec![output.clone(), assets]
            } else {
                vec![assets]
            }
        );
        settings.assets_dir = None;
        let report = export_saved_chat(&[], &settings).unwrap();
        assert_eq!(report.output_dir, output);
        assert!(report.html_pages.iter().all(|path| path.is_file()));
        assert!(report.json_path.unwrap().is_file());
    }
}

#[test]
fn archive_late_media_failure_cleans_up_already_copied_plaintext() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("media");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("first.bin"), b"first").unwrap();
    fs::create_dir(root.join("not-a-file")).unwrap();
    let mut settings = options(directory.path().join("archive"));
    settings.media_root = Some(root.clone());
    let records = [message(1, "first.bin"), message(2, "not-a-file")];
    assert!(export_saved_chat(&records, &settings).is_err());
    assert!(!settings.output_dir.exists());
    assert_eq!(entries(directory.path()), [root]);
    let report = export_saved_chat(&records[..1], &settings).unwrap();
    assert_eq!(report.copied_media, 1);
    assert_eq!(
        fs::read(settings.output_dir.join("files/first.bin")).unwrap(),
        b"first"
    );
}

#[test]
fn archive_refuses_existing_content_without_changing_it() {
    let directory = tempdir().unwrap();
    let settings = options(directory.path().join("archive"));
    fs::create_dir(&settings.output_dir).unwrap();
    let retained = settings.output_dir.join("retained.bin");
    fs::write(&retained, b"preserve this archive").unwrap();
    assert!(export_saved_chat(&[], &settings).is_err());
    assert_eq!(
        entries(&settings.output_dir),
        std::slice::from_ref(&retained)
    );
    assert_eq!(fs::read(retained).unwrap(), b"preserve this archive");
    assert_eq!(entries(directory.path()), [settings.output_dir]);
}

#[test]
fn json_archive_accepts_directory_path_suffixes_and_returns_published_paths() {
    for suffix in ["", "/", "/.", "/./"] {
        for preexisting in [false, true] {
            let directory = tempdir().unwrap();
            let mut path = directory.path().join("nested/archive").into_os_string();
            path.push(suffix);
            let mut settings = options(PathBuf::from(path));
            settings.format = ExportFormat::Json;
            settings.messages_per_page = 0;
            if preexisting {
                fs::create_dir_all(directory.path().join("nested/archive")).unwrap();
            }
            let report = export_saved_chat(&[], &settings).unwrap();
            assert!(report.html_pages.is_empty());
            assert_eq!(report.output_dir, settings.output_dir);
            assert_eq!(
                report.json_path,
                Some(settings.output_dir.join("result.json"))
            );
            assert!(report.json_path.unwrap().is_file());
            assert_eq!(entries(&directory.path().join("nested")).len(), 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn archive_refuses_escaping_media_and_output_symlinks_without_partial_files() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let root = directory.path().join("media");
    let outside = directory.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("first.bin"), b"first").unwrap();
    fs::write(outside.join("hidden.bin"), b"hidden").unwrap();
    symlink(outside.join("hidden.bin"), root.join("escape.bin")).unwrap();
    let mut settings = options(directory.path().join("archive"));
    settings.media_root = Some(root.clone());
    assert!(
        export_saved_chat(
            &[message(1, "first.bin"), message(2, "escape.bin")],
            &settings
        )
        .is_err()
    );
    assert!(!settings.output_dir.exists());
    assert_eq!(fs::read(outside.join("hidden.bin")).unwrap(), b"hidden");
    symlink(&outside, &settings.output_dir).unwrap();
    assert!(export_saved_chat(&[], &settings).is_err());
    assert_eq!(entries(&outside), [outside.join("hidden.bin")]);
}

#[test]
fn archive_keeps_literal_unicode_filenames_and_encodes_html_urls() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("media");
    fs::create_dir(&root).unwrap();
    let name = "فایل #100%.bin";
    fs::write(root.join(name), b"first").unwrap();
    let mut settings = options(directory.path().join("archive"));
    settings.media_root = Some(root);
    let report = export_saved_chat(&[message(1, name)], &settings).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(report.json_path.unwrap()).unwrap()).unwrap();
    assert_eq!(json["messages"][0]["file"], format!("files/{name}"));
    assert_eq!(
        fs::read(settings.output_dir.join(format!("files/{name}"))).unwrap(),
        b"first"
    );
    let html = fs::read_to_string(&report.html_pages[0]).unwrap();
    assert!(html.contains("%23"));
    assert!(html.contains("%25.bin"));
    assert!(!html.contains(&format!("href=\"files/{name}")));
}

fn reference() -> ForwardMediaReference {
    let key = [7; 32];
    let iv = [9; 32];
    ForwardMediaReference {
        kind: SecretMediaKind::Document,
        metadata: MediaMetadata::default(),
        file_id: -51,
        access_hash: -81,
        dc_id: 2,
        size_bytes: 1,
        key: EncryptedMediaKey {
            key,
            iv,
            fingerprint: compute_file_key_fingerprint(&key, &iv),
        },
    }
}

#[test]
fn forwarding_refuses_unusable_location_and_size_before_building_a_descriptor() {
    let original = reference();
    let mut invalid = original.clone();
    invalid.file_id = 0;
    assert!(build_forward_media_descriptor(&invalid, None).is_err());
    for dc_id in [i32::MIN, -1, 0] {
        invalid = original.clone();
        invalid.dc_id = dc_id;
        assert!(build_forward_media_descriptor(&invalid, None).is_err());
    }
    for size in [0, 4 * 1024 * 1024 * 1024 + 1, u64::MAX] {
        invalid = original.clone();
        invalid.size_bytes = size;
        assert!(build_forward_media_descriptor(&invalid, None).is_err());
    }
    for size in [
        1,
        16,
        17,
        4 * 1024 * 1024 * 1024 - 1,
        4 * 1024 * 1024 * 1024,
    ] {
        let mut valid = original.clone();
        valid.size_bytes = size;
        let result = build_forward_media_descriptor(&valid, Some("caption".into())).unwrap();
        assert_eq!(result.file_id, original.file_id);
        assert_eq!(result.access_hash, original.access_hash);
        assert_eq!(result.size_bytes, size);
        assert_eq!(result.key.key, original.key.key);
        assert_eq!(result.key.iv, original.key.iv);
        assert_eq!(result.key.fingerprint, original.key.fingerprint);
    }
}

fn ogg_page(packet: &[u8], sequence: u32) -> Vec<u8> {
    let mut page = vec![0; 27];
    page[..4].copy_from_slice(b"OggS");
    page[5] = if sequence == 0 { 2 } else { 0 };
    page[18..22].copy_from_slice(&sequence.to_le_bytes());
    page[26] = u8::try_from(packet.len() / 255 + 1).unwrap();
    page.extend(std::iter::repeat_n(255, packet.len() / 255));
    page.push((packet.len() % 255) as u8);
    page.extend(packet);
    let mut checksum = 0_u32;
    for byte in &page {
        checksum ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            checksum = (checksum << 1)
                ^ if checksum & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
        }
    }
    page[22..26].copy_from_slice(&checksum.to_le_bytes());
    page
}

fn opus_prefix(identification: &[u8]) -> Vec<u8> {
    let mut bytes = ogg_page(identification, 0);
    bytes.extend(ogg_page(b"OpusTags\0\0\0\0\0\0\0\0", 1));
    bytes
}

#[test]
fn voice_detection_requires_a_complete_opus_identification_packet() {
    let header = b"OpusHead\x01\x01\x00\x00\x80\xbb\x00\x00\x00\x00\x00";
    for end in 0..header.len() {
        assert!(
            !detect_voice_note(&opus_prefix(&header[..end]), "audio/ogg"),
            "truncated ID header of {end} bytes"
        );
    }
    for version in [0, 1, 15] {
        let mut valid = header.to_vec();
        valid[8] = version;
        assert!(detect_voice_note(&opus_prefix(&valid), "audio/ogg"));
    }
    let mut stereo = header.to_vec();
    stereo[9] = 2;
    assert!(detect_voice_note(&opus_prefix(&stereo), "audio/opus"));
    for (offset, value) in [(8, 16), (8, 255), (9, 0), (9, 3)] {
        let mut invalid = header.to_vec();
        invalid[offset] = value;
        assert!(!detect_voice_note(&opus_prefix(&invalid), "audio/ogg"));
    }
}

#[test]
fn voice_detection_checks_nonzero_mapping_fields_and_retains_valid_multichannel_headers() {
    let mut mapped = b"OpusHead\x01\x03\x00\x00\x80\xbb\x00\x00\x00\x00\x01".to_vec();
    mapped.extend([2, 1, 0, 2, 1]);
    assert!(detect_voice_note(&opus_prefix(&mapped), "audio/ogg"));
    for end in 19..mapped.len() {
        assert!(!detect_voice_note(
            &opus_prefix(&mapped[..end]),
            "audio/ogg"
        ));
    }
    for (offset, value) in [(19, 0), (20, 3), (21, 3)] {
        let mut invalid = mapped.clone();
        invalid[offset] = value;
        assert!(!detect_voice_note(&opus_prefix(&invalid), "audio/ogg"));
    }
}

struct SmallReads<'a>(Cursor<&'a [u8]>);

impl Read for SmallReads<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = out.len().min(3);
        self.0.read(&mut out[..count])
    }
}

#[test]
fn streaming_crypto_accepts_short_reads_and_matches_independent_ige_at_chunk_boundaries() {
    for size in [0_usize, 1, 15, 16, 17, 65_535, 65_536, 65_537, 131_073] {
        let plaintext: Vec<_> = (0..size).map(|i| (i % 251) as u8).collect();
        let mut encrypted = Vec::new();
        let key = encrypt_media_stream(
            &mut SmallReads(Cursor::new(&plaintext)),
            &mut encrypted,
            size as u64,
        )
        .unwrap();
        assert_eq!(encrypted.len(), size.div_ceil(16) * 16);
        let mut independent = encrypted.clone();
        kiaro_secret_chat::grammers_crypto::aes::ige_decrypt(&mut independent, &key.key, &key.iv);
        assert_eq!(&independent[..size], &plaintext);
        let mut restored = Vec::new();
        decrypt_media_stream(
            &mut SmallReads(Cursor::new(&encrypted)),
            &mut restored,
            encrypted.len() as u64,
            size as u64,
            &key,
        )
        .unwrap();
        assert_eq!(restored, plaintext);
        let mut too_long = encrypted.clone();
        too_long.push(0);
        assert!(
            decrypt_media_stream(
                &mut Cursor::new(too_long),
                &mut Vec::new(),
                encrypted.len() as u64,
                size as u64,
                &key
            )
            .is_err()
        );
        if !encrypted.is_empty() {
            assert!(
                decrypt_media_stream(
                    &mut Cursor::new(&encrypted[..encrypted.len() - 1]),
                    &mut Vec::new(),
                    encrypted.len() as u64,
                    size as u64,
                    &key
                )
                .is_err()
            );
        }
    }
}
