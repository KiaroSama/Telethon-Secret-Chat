#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    assets::ASSET_NAMES,
    error::{ExportError, Result},
    html_writer::HtmlWriter,
    json_writer::JsonWriter,
    model::*,
    model_dialogs::{DialogInfo, DialogType, DialogsInfo},
    model_message::{Message, MessagesSlice},
    model_peers::{ContactInfo, Peer, PeerUser, Peers},
    output_file::OutputFile,
    settings::{Environment, Format, Settings},
    writer::{ExportWriter, HtmlAndJsonWriter},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
const DATE: i64 = 86400 * 365;
const D: i64 = 1705053605;
fn settings(folder: &Path, peer: Option<i64>) -> Settings {
    Settings {
        path: folder.into(),
        single_peer: peer,
        time_zone: Some("UTC".into()),
        ..Default::default()
    }
}
fn peers(id: i64, first: &str, last: &str) -> Peers {
    [(
        id,
        Peer::User(PeerUser {
            bare_id: id,
            info: ContactInfo {
                user_id: id,
                first_name: first.into(),
                last_name: last.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )]
    .into_iter()
    .collect()
}
fn message(id: i64, date: i64, text: &str) -> Message {
    Message {
        id,
        date,
        from_id: 5,
        text: vec![TextPart::plain(text)],
        ..Default::default()
    }
}
fn export(
    writer: &mut dyn ExportWriter,
    s: &Settings,
    dialog: &DialogInfo,
    slices: &[MessagesSlice],
) {
    writer
        .start(
            s,
            &Environment {
                internal_links_domain: "t/".into(),
                ..Default::default()
            },
        )
        .unwrap();
    writer
        .write_dialogs_start(&DialogsInfo {
            chats: vec![dialog.clone()],
            ..Default::default()
        })
        .unwrap();
    writer.write_dialog_start(dialog).unwrap();
    for slice in slices {
        writer.write_dialog_slice(slice).unwrap();
    }
    writer.write_dialog_end().unwrap();
    writer.write_dialogs_end().unwrap();
    writer.finish().unwrap();
}
fn html_export(
    folder: &Path,
    slices: Vec<Vec<Message>>,
    s: Option<Settings>,
    dialog: Option<DialogInfo>,
) -> HtmlWriter {
    let mut writer = HtmlWriter::new();
    let slices = slices
        .into_iter()
        .map(|list| MessagesSlice {
            list,
            peers: peers(5, "Ann", "Lee"),
        })
        .collect::<Vec<_>>();
    export(
        &mut writer,
        &s.unwrap_or_else(|| settings(folder, Some(5))),
        &dialog.unwrap_or(DialogInfo {
            name: "Ann".into(),
            peer_id: 5,
            ..Default::default()
        }),
        &slices,
    );
    writer
}
fn actual_pages(root: &Path, folder: &Path, out: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            actual_pages(root, &path, out);
        } else if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("html" | "json")
        ) {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|p| p.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(name, hex::encode(fs::read(path).unwrap()));
        }
    }
}
fn compare(folder: &Path, case: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tdexport_writers")
        .join(format!("{case}.json"));
    let expected: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    let mut actual = BTreeMap::new();
    actual_pages(folder, folder, &mut actual);
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        expected["files"],
        "{}",
        expected["legacy_case"]
    );
}
#[test]
fn single_message_page_matches_tdesktop() {
    let folder = tempfile::tempdir().unwrap();
    let writer = html_export(
        folder.path(),
        vec![vec![message(1, DATE, "Hi & bye")]],
        None,
        None,
    );
    assert_eq!(writer.main_file_path(), folder.path().join("messages.html"));
    compare(folder.path(), "single_message_page_matches_tdesktop");
}
#[test]
fn assets_are_copied_like_tdesktop() {
    let folder = tempfile::tempdir().unwrap();
    html_export(
        folder.path(),
        vec![vec![message(1, DATE, "Hi")]],
        None,
        None,
    );
    let hashes: BTreeMap<String, String> = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tdexport_assets_sha256_current.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(hashes.len(), ASSET_NAMES.len());
    for (name, expected) in hashes {
        assert_eq!(
            hex::encode(Sha256::digest(fs::read(folder.path().join(name)).unwrap())),
            expected
        );
    }
    let mut count = 0;
    fn count_files(folder: &Path, count: &mut usize) {
        for e in fs::read_dir(folder).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                count_files(&p, count);
            } else {
                *count += 1;
            }
        }
    }
    count_files(folder.path(), &mut count);
    assert_eq!(count, ASSET_NAMES.len() + 1);
}
#[test]
fn paging_links_and_cross_page_replies() {
    let folder = tempfile::tempdir().unwrap();
    let first = (1..=1000)
        .map(|id| message(id, DATE, "Hi"))
        .collect::<Vec<_>>();
    let mut second = first[600..].to_vec();
    second.push(Message {
        reply_to_msg_id: 1,
        ..message(1001, DATE, "Hi")
    });
    second.push(Message {
        reply_to_msg_id: 1001,
        ..message(1002, DATE, "Hi")
    });
    html_export(
        folder.path(),
        vec![first[..600].to_vec(), second],
        None,
        None,
    );
    let page1 = fs::read_to_string(folder.path().join("messages.html")).unwrap();
    let page2 = fs::read_to_string(folder.path().join("messages2.html")).unwrap();
    assert_eq!(
        page1.matches("class=\"message default clearfix").count(),
        1000
    );
    assert!(page1.contains("href=\"messages2.html\""));
    assert!(page2.contains("href=\"messages.html#go_to_message1\""));
    assert!(page2.contains("href=\"#go_to_message1001\""));
    assert_eq!(
        page2.matches("class=\"message default clearfix\"").count(),
        1
    );
    assert!(!folder.path().join("messages3.html").exists());
    compare(folder.path(), "paging_links_and_cross_page_replies");
}
#[test]
fn empty_and_date_limited_exports() {
    let folder = tempfile::tempdir().unwrap();
    html_export(
        folder.path(),
        vec![],
        None,
        Some(DialogInfo {
            peer_id: 5,
            ..Default::default()
        }),
    );
    let limited = folder.path().join("limited");
    let mut s = settings(&limited, Some(5));
    s.single_peer_from = DATE + 10;
    s.single_peer_till = DATE + 20;
    html_export(
        &limited,
        vec![vec![
            message(1, DATE, "Hi"),
            message(2, DATE + 15, "Hi"),
            message(3, DATE + 20, "Hi"),
        ]],
        Some(s),
        None,
    );
    compare(folder.path(), "empty_and_date_limited_exports");
}
#[test]
fn existing_page_is_truncated_and_bytes_are_kept() {
    let folder = tempfile::tempdir().unwrap();
    fs::write(folder.path().join("messages.html"), vec![b'x'; 100000]).unwrap();
    let mention = TextPart {
        kind: TextType::Mention,
        text: "ét".into(),
        additional: String::new(),
    };
    html_export(
        folder.path(),
        vec![vec![Message {
            text: vec![mention],
            ..message(1, DATE, "")
        }]],
        None,
        None,
    );
    let bytes = fs::read(folder.path().join("messages.html")).unwrap();
    assert!(bytes.starts_with(b"<!DOCTYPE html>"));
    assert!(
        bytes
            .windows(b"<a href=\"t/\xa9t\">\xc3\xa9t</a>".len())
            .any(|w| w == b"<a href=\"t/\xa9t\">\xc3\xa9t</a>")
    );
    compare(
        folder.path(),
        "existing_page_is_truncated_and_bytes_are_kept",
    );
}
#[test]
fn account_wide_export_is_not_ported() {
    let folder = tempfile::tempdir().unwrap();
    assert_eq!(
        HtmlWriter::new().start(&settings(folder.path(), None), &Environment::default()),
        Err(ExportError::Unsupported)
    );
    assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
}
#[test]
fn single_chat_result_json_and_date_skip() {
    let folder = tempfile::tempdir().unwrap();
    let mut s = settings(folder.path(), Some(100));
    s.single_peer_from = D;
    let dialog = DialogInfo {
        r#type: DialogType::Personal,
        name: "Ann".into(),
        peer_id: 100,
        ..Default::default()
    };
    let early = Message {
        id: 4,
        date: D - 1,
        from_id: 100,
        text: vec![TextPart::plain("old")],
        ..Default::default()
    };
    let kept = Message {
        id: 5,
        date: D,
        from_id: 100,
        text: vec![TextPart::plain("hi")],
        ..Default::default()
    };
    let mut writer = JsonWriter::new();
    export(
        &mut writer,
        &s,
        &dialog,
        &[
            MessagesSlice {
                list: vec![early, kept],
                peers: peers(100, "Ann", ""),
            },
            MessagesSlice::default(),
        ],
    );
    assert_eq!(writer.main_file_path(), folder.path().join("result.json"));
    compare(folder.path(), "single_chat_result_json_and_date_skip");
}
#[test]
fn saved_messages_has_no_name_and_empty_list() {
    let folder = tempfile::tempdir().unwrap();
    export(
        &mut JsonWriter::new(),
        &settings(folder.path(), Some(100)),
        &DialogInfo {
            r#type: DialogType::Self_,
            name: "Me".into(),
            peer_id: 100,
            ..Default::default()
        },
        &[],
    );
    compare(folder.path(), "saved_messages_has_no_name_and_empty_list");
}
#[test]
fn account_wide_framing_when_not_single_peer() {
    let folder = tempfile::tempdir().unwrap();
    let mut writer = JsonWriter::new();
    writer
        .start(
            &settings(folder.path(), None),
            &Environment {
                about_telegram: "About".into(),
                about_chats: "Chats".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let info = DialogInfo {
        r#type: DialogType::Personal,
        name: "Ann".into(),
        peer_id: 100,
        ..Default::default()
    };
    writer
        .write_dialogs_start(&DialogsInfo {
            chats: vec![info.clone()],
            ..Default::default()
        })
        .unwrap();
    writer.write_dialog_start(&info).unwrap();
    writer
        .write_dialog_slice(&MessagesSlice::default())
        .unwrap();
    writer.write_dialog_end().unwrap();
    writer.write_dialogs_end().unwrap();
    writer.finish().unwrap();
    compare(folder.path(), "account_wide_framing_when_not_single_peer");
}
#[test]
fn output_file_creates_folder_and_truncates() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("sub dir/result.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"stale content").unwrap();
    let mut output = OutputFile::new(&path);
    output.write_block(b"").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"");
    output.write_block(b"ab").unwrap();
    output.write_block(b"c").unwrap();
    output.close();
    assert_eq!(fs::read(&path).unwrap(), b"abc");
    assert_eq!(output.size(), 3);
    fs::write(&path, b"abc stale tail").unwrap();
    output.write_block(b"d").unwrap();
    output.close();
    assert_eq!(fs::read(&path).unwrap(), b"abcd");
    assert_eq!(output.size(), 4);
    let mut fresh = OutputFile::new(folder.path().join("new/x.json"));
    fresh.write_block(b"1").unwrap();
    fresh.close();
    assert_eq!(fs::read(folder.path().join("new/x.json")).unwrap(), b"1");
}
struct Recorder {
    name: &'static str,
    calls: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
    failure: Option<ExportError>,
}
impl Recorder {
    fn call(&self, method: &'static str) -> Result<()> {
        self.calls.lock().unwrap().push((self.name, method));
        self.failure.map_or(Ok(()), Err)
    }
}
impl ExportWriter for Recorder {
    fn format(&self) -> Format {
        Format::Html
    }
    fn start(&mut self, _: &Settings, _: &Environment) -> Result<()> {
        self.call("start")
    }
    fn write_dialogs_start(&mut self, _: &DialogsInfo) -> Result<()> {
        self.call("write_dialogs_start")
    }
    fn write_dialog_start(&mut self, _: &DialogInfo) -> Result<()> {
        self.call("write_dialog_start")
    }
    fn write_dialog_slice(&mut self, _: &MessagesSlice) -> Result<()> {
        self.call("write_dialog_slice")
    }
    fn write_dialog_end(&mut self) -> Result<()> {
        self.call("write_dialog_end")
    }
    fn write_dialogs_end(&mut self) -> Result<()> {
        self.call("write_dialogs_end")
    }
    fn finish(&mut self) -> Result<()> {
        self.call("finish")
    }
    fn main_file_path(&self) -> PathBuf {
        format!("{}.path", self.name).into()
    }
}
#[test]
fn html_and_json_calls_both_and_reports_the_last_failure() {
    let calls = Arc::new(Mutex::new(vec![]));
    let mut writer = HtmlAndJsonWriter::new(
        Box::new(Recorder {
            name: "html",
            calls: calls.clone(),
            failure: Some(ExportError::Filesystem),
        }),
        Box::new(Recorder {
            name: "json",
            calls: calls.clone(),
            failure: None,
        }),
    );
    assert_eq!(writer.write_dialog_end(), Err(ExportError::Filesystem));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("html", "write_dialog_end"), ("json", "write_dialog_end")]
    );
    assert_eq!(writer.main_file_path(), PathBuf::from("html.path"));
}
