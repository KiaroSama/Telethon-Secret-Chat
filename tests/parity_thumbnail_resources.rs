#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

use std::{env, fmt::Display, fs, path::Path, process::Command};

use kiaro_secret_chat::{
    autosave::{SavedMediaRecord, SavedSecretMessage},
    export_archive::{ExportFormat, ExportMediaFilter, ExportOptions, export_saved_chat},
    media::SecretMediaKind,
    ported::tdexport_qt_jpeg::desktop_thumb,
};
use mozjpeg::{ColorSpace, Compress, Decompress};
use tempfile::tempdir;

const WIDTH: usize = 6000;
const HEIGHT: usize = 4000;
const THUMB_WIDTH: usize = 320;
const THUMB_HEIGHT: usize = 213;
const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
const CHILD_CASE: &str = "KIARO_THUMBNAIL_RESOURCE_CASE";
const CHILD_ROOT: &str = "KIARO_THUMBNAIL_RESOURCE_ROOT";

fn log(level: &str, message: impl Display) {
    assert!(matches!(level, "INFO" | "WARNING" | "ERROR" | "DEBUG"));
    eprintln!("{} [{level}] {message}", chrono::Utc::now().to_rfc3339());
}

fn solid_jpeg() -> Vec<u8> {
    let mut encoder = Compress::new(ColorSpace::JCS_RGB);
    encoder.set_size(WIDTH, HEIGHT);
    encoder.set_quality(75.0);
    let mut encoder = encoder.start_compress(Vec::new()).unwrap();
    let pixels = [90, 140, 200].repeat(WIDTH * HEIGHT);
    encoder.write_scanlines(&pixels).unwrap();
    drop(pixels);
    let jpeg = encoder.finish().unwrap();
    assert!((jpeg.len() as u64) < SOURCE_LIMIT);
    assert_eq!(Decompress::new_mem(&jpeg).unwrap().size(), (WIDTH, HEIGHT));
    log(
        "DEBUG",
        format!("Generated a valid 24 MP RGB JPEG ({} bytes)", jpeg.len()),
    );
    jpeg
}

fn assert_thumbnail(bytes: &[u8]) {
    let input = Decompress::new_mem(bytes).unwrap();
    assert_eq!(input.size(), (THUMB_WIDTH, THUMB_HEIGHT));
    let mut decoder = input.to_colorspace(ColorSpace::JCS_RGB).unwrap();
    let pixels = decoder.read_scanlines::<u8>().unwrap();
    decoder.finish().unwrap();
    assert_eq!(pixels.len(), THUMB_WIDTH * THUMB_HEIGHT * 3);
    assert!(pixels.as_chunks::<3>().0.iter().all(|pixel| {
        pixel
            .iter()
            .zip([90_u8, 140, 200])
            .all(|(&actual, expected)| actual.abs_diff(expected) <= 4)
    }));
}

fn exercise_archive(root: &Path, jpeg: &[u8]) {
    let media_root = root.join("media");
    fs::create_dir(&media_root).unwrap();
    fs::write(media_root.join("large.jpg"), jpeg).unwrap();
    let output_dir = root.join("archive");
    let options = ExportOptions {
        chat_id: 7,
        chat_title: "Thumbnail resource acceptance".into(),
        peer_id: 9,
        format: ExportFormat::HtmlAndJson,
        output_dir: output_dir.clone(),
        assets_dir: None,
        media_root: Some(media_root),
        media_filter: ExportMediaFilter {
            photos: true,
            videos: true,
            voice_messages: true,
            video_messages: true,
            stickers: true,
            gifs: true,
            files: true,
            size_limit_bytes: SOURCE_LIMIT,
        },
        date_from_unix: None,
        date_to_unix: None,
        messages_per_page: 10,
    };
    let message = SavedSecretMessage {
        id: 1,
        chat_id: 7,
        sender_id: 9,
        sender_name: "Synthetic photo sender".into(),
        outgoing: false,
        date_unix: 1_700_000_000,
        text: "A normal-resolution photo must not exhaust thumbnail memory".into(),
        reply_to_msg_id: None,
        ttl_seconds: None,
        media: Some(SavedMediaRecord {
            kind: SecretMediaKind::Photo,
            mime_type: "image/jpeg".into(),
            file_name: "large.jpg".into(),
            size_bytes: jpeg.len() as u64,
            local_rel_path: Some("large.jpg".into()),
            width: Some(WIDTH as u32),
            height: Some(HEIGHT as u32),
            duration_secs: None,
        }),
    };
    let report = export_saved_chat(&[message], &options).unwrap();
    assert_eq!(
        (
            report.exported_messages,
            report.copied_media,
            report.skipped_media
        ),
        (1, 1, 0)
    );
    assert_eq!(fs::read(output_dir.join("photos/large.jpg")).unwrap(), jpeg);
    let thumbnail = fs::read(output_dir.join("photos/photo_1_thumb.jpg")).unwrap();
    assert_thumbnail(&thumbnail);
    assert_eq!(report.html_pages.len(), 1);
    assert!(
        fs::read_to_string(&report.html_pages[0])
            .unwrap()
            .contains("photos/photo_1_thumb.jpg")
    );
    assert!(report.json_path.unwrap().is_file());
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}

fn run_bounded(test_name: &str, case: &str) {
    if env::var(CHILD_CASE).as_deref() == Ok(case) {
        log(
            "INFO",
            format!("Running {case} thumbnail path within the 512 MiB child budget"),
        );
        let jpeg = solid_jpeg();
        match case {
            "direct" => {
                let thumbnail = desktop_thumb(jpeg, THUMB_WIDTH as i64, THUMB_HEIGHT as i64, None)
                    .unwrap()
                    .expect("a valid 24 MP photo must retain thumbnail support");
                assert_thumbnail(&thumbnail);
            }
            "archive" => exercise_archive(Path::new(&env::var_os(CHILD_ROOT).unwrap()), &jpeg),
            _ => panic!("unknown thumbnail resource case"),
        }
        log(
            "INFO",
            format!("The {case} thumbnail path completed successfully"),
        );
        return;
    }

    // The parent owns cleanup even if an allocation failure aborts the child.
    // Apply limits before starting Rust; never constrain the main test process.
    let directory = tempdir().unwrap();
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "set -eu\nulimit -c 0\nulimit -v 524288\nulimit -t 120\nexec \"$@\"",
            "thumbnail-resource-child",
        ])
        .arg(env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, case)
        .env(CHILD_ROOT, directory.path())
        .output()
        .unwrap();
    if !output.status.success() {
        log(
            "ERROR",
            format!("The capped {case} child failed: {}", output.status),
        );
    }
    assert!(
        output.status.success(),
        "{case} thumbnail child failed within the 512 MiB budget: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn desktop_thumbnail_handles_a_normal_photo_with_bounded_memory() {
    run_bounded(
        "desktop_thumbnail_handles_a_normal_photo_with_bounded_memory",
        "direct",
    );
}

#[test]
fn archive_thumbnail_handles_a_normal_photo_with_bounded_memory() {
    run_bounded(
        "archive_thumbnail_handles_a_normal_photo_with_bounded_memory",
        "archive",
    );
}
