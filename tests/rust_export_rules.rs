#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    files::*,
    format::*,
    model::{Document, File, FileLocation, SkipReason},
    settings::{Format, MediaSettings, Settings},
};
use serde_json::json;
use std::fs;
#[test]
fn settings_defaults_match_tdesktop() {
    let settings = Settings::default();
    assert_eq!(settings.media.types, MediaSettings::PHOTO);
    assert_eq!(settings.media.size_limit, 8 * 1024 * 1024);
    assert_eq!(settings.format, Format::Html);
    assert!(!settings.only_single_peer());
    assert!(settings.validate());
    let single = Settings {
        single_peer: Some(0),
        ..Default::default()
    }
    .normalized();
    assert_eq!(
        (single.types, single.full_chats),
        (Settings::ANY_CHATS_MASK, Settings::ANY_CHATS_MASK)
    );
    assert!(
        !Settings {
            single_peer_from: 10,
            single_peer_till: 5,
            ..Default::default()
        }
        .validate()
    );
    assert!(
        !MediaSettings {
            size_limit: 4001 * 1024 * 1024,
            ..Default::default()
        }
        .validate()
    );
}
#[test]
fn mime_first_glob_uses_tdesktop_known_types_and_qt_database() {
    assert_eq!(mime_first_glob("image/webp"), "*.webp");
    assert_eq!(mime_first_glob("application/x-tgsticker"), "*.tgs");
    assert_eq!(mime_first_glob("video/mp4"), "*.mp4");
    assert_eq!(mime_first_glob("image/jpeg"), "*.jpg");
    assert_eq!(
        mime_first_glob("audio/mpeg3"),
        mime_first_glob("audio/mpeg")
    );
    assert!(!mime_first_glob("audio/mpeg").is_empty());
    assert_eq!(mime_first_glob("application/unknown-thing"), "");
}
#[test]
fn prepare_relative_path_adds_counter_before_first_dot() {
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(
        prepare_relative_path(folder.path(), "files/a.tar.gz"),
        "files/a.tar.gz"
    );
    fs::create_dir(folder.path().join("files")).unwrap();
    fs::write(folder.path().join("files/a.tar.gz"), b"").unwrap();
    assert_eq!(
        prepare_relative_path(folder.path(), "files/a.tar.gz"),
        "files/a (1).tar.gz"
    );
    fs::write(folder.path().join("files/a (1).tar.gz"), b"").unwrap();
    assert_eq!(
        prepare_relative_path(folder.path(), "files/a.tar.gz"),
        "files/a (2).tar.gz"
    );
}
#[test]
fn normalize_path_uses_chat_export_folder() {
    let folder = tempfile::tempdir().unwrap();
    let single = Settings {
        single_peer: Some(0),
        path: folder.path().into(),
        ..Default::default()
    };
    let dates = DateContext::utc();
    assert_eq!(
        normalize_path(&single, Some("2026-09-30"), &dates).unwrap(),
        folder.path()
    );
    fs::write(folder.path().join("other.txt"), "x").unwrap();
    assert_eq!(
        normalize_path(&single, Some("2026-09-30"), &dates).unwrap(),
        folder.path().join("ChatExport_2026-09-30")
    );
    fs::create_dir(folder.path().join("ChatExport_2026-09-30")).unwrap();
    assert_eq!(
        normalize_path(&single, Some("2026-09-30"), &dates).unwrap(),
        folder.path().join("ChatExport_2026-09-30 (1)")
    );
    let forced = Settings {
        path: folder.path().join("empty"),
        force_sub_path: true,
        ..Default::default()
    };
    assert_eq!(
        normalize_path(&forced, Some("2026-01-02"), &dates).unwrap(),
        folder.path().join("empty/DataExport_2026-01-02")
    );
}
#[test]
fn apply_file_policy_order() {
    let media = MediaSettings {
        types: MediaSettings::PHOTO,
        size_limit: 100,
    };
    let location = FileLocation {
        dc_id: 1,
        data: json!({"_":"InputPhotoFileLocation","id":1,"access_hash":1,"file_reference":[],"thumb_size":"y"}),
    };
    let decide = |mut file: File, skip: bool, kind: u32, size: u64| {
        let done = apply_file_policy(&mut file, skip, kind, size, &media);
        (done, file.skip_reason)
    };
    assert_eq!(
        decide(
            File {
                location: location.clone(),
                ..Default::default()
            },
            true,
            MediaSettings::PHOTO,
            1
        ),
        (true, SkipReason::DateLimits)
    );
    assert_eq!(
        decide(File::default(), false, MediaSettings::PHOTO, 1),
        (true, SkipReason::Unavailable)
    );
    assert_eq!(
        decide(
            File {
                content: b"x".to_vec(),
                ..Default::default()
            },
            false,
            MediaSettings::VIDEO,
            1
        ),
        (true, SkipReason::FileType)
    );
    assert_eq!(
        decide(
            File {
                location: location.clone(),
                ..Default::default()
            },
            false,
            MediaSettings::PHOTO,
            101
        ),
        (true, SkipReason::FileSize)
    );
    assert_eq!(
        decide(
            File {
                location,
                ..Default::default()
            },
            false,
            MediaSettings::PHOTO,
            1
        ),
        (false, SkipReason::None)
    );
    assert_eq!(
        decide(
            File {
                relative_path: "done".into(),
                ..Default::default()
            },
            false,
            MediaSettings::PHOTO,
            1
        ),
        (true, SkipReason::None)
    );
    assert_eq!(
        Document {
            is_sticker: true,
            is_animated: true,
            ..Default::default()
        }
        .media_type(),
        MediaSettings::STICKER
    );
    assert_eq!(
        Document {
            is_animated: true,
            ..Default::default()
        }
        .media_type(),
        MediaSettings::GIF
    );
}
#[test]
fn formatting_helpers() {
    let stamp = 1705053605;
    let dates = DateContext::utc();
    assert_eq!(
        format_date_time(stamp, false, ".", ":", " ", &dates).unwrap(),
        "12.01.2024 10:00:05"
    );
    assert_eq!(
        format_date_time(stamp, true, ".", ":", " ", &dates).unwrap(),
        "12.01.2024 10:00:05 UTC+00:00"
    );
    assert_eq!(
        format_date_time(stamp, false, "-", "-", "_", &dates).unwrap(),
        "12-01-2024_10-00-05"
    );
    assert_eq!(
        format_date_time(0, false, ".", ":", " ", &dates).unwrap(),
        ""
    );
    assert_eq!(
        [512, 1536, 5 * 1024 * 1024 + 1].map(format_file_size),
        ["512 B", "1.5 KB", "5.0 MB"]
    );
    assert_eq!(
        [5, 65, 3725].map(format_duration),
        ["00:05", "01:05", "1:02:05"]
    );
    assert_eq!(number_to_string(7, 3, "0"), "007");
    for (amount, currency, expected) in [
        (123456, "USD", "$1,234.56"),
        (-500, "JPY", "−¥500"),
        (100000, "DKK", "1000,00 DKK"),
        (1500, "XTR", "⭐1,500"),
        (700, "ZZZ", "ZZZ7.00"),
    ] {
        assert_eq!(fill_amount_and_currency(amount, currency, false), expected);
    }
}
#[test]
fn phone_formatting_uses_server_country_list() {
    assert_eq!(format_phone_number("79991234567", &[]), "79991234567");
    let countries = [CountryInfo {
        name: "Russia".into(),
        iso2: "RU".into(),
        codes: vec![CallingCodeInfo {
            calling_code: "7".into(),
            prefixes: vec![String::new()],
            patterns: vec!["XXX XXX XXXX".into()],
        }],
        ..Default::default()
    }];
    assert_eq!(
        format_phone_number("+7 9991234567", &countries),
        "+7 999 123 4567"
    );
    assert_eq!(format_phone_number("0123", &countries), "0123");
    assert_eq!(format_phone_number("", &countries), "");
}
#[test]
fn file_name_from_user_string_0() {
    assert_eq!(
        file_name_from_user_string(r###"report.pdf"###),
        r###"report.pdf"###
    );
}
#[test]
fn file_name_from_user_string_1() {
    assert_eq!(
        file_name_from_user_string(r###"a<b>c:"d|e?f*g\h/i"###),
        r###"a_b_c__d_e_f_g_h_i"###
    );
}
#[test]
fn file_name_from_user_string_2() {
    assert_eq!(
        file_name_from_user_string("x\u{202e}gnp.exe"),
        r###"x_gnp.exe"###
    );
}
#[test]
fn file_name_from_user_string_3() {
    assert_eq!(
        file_name_from_user_string(r###"trailing."###),
        r###"trailing._"###
    );
}
#[test]
fn file_name_from_user_string_4() {
    assert_eq!(file_name_from_user_string(r###""###), r###"_"###);
}
#[test]
fn file_name_from_user_string_5() {
    assert_eq!(
        file_name_from_user_string(r###"con.txt"###),
        r###"_con.txt"###
    );
}
#[test]
fn file_name_from_user_string_6() {
    assert_eq!(
        file_name_from_user_string(r###"CONSOLE.txt"###),
        r###"CONSOLE.txt"###
    );
}
#[test]
fn file_name_from_user_string_7() {
    assert_eq!(
        file_name_from_user_string(r###"link.lnk"###),
        r###"link.lnk.download"###
    );
}
