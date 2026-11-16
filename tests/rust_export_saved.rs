#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::export::{
    self, ExportError, ExportWriter, Format, MediaSettings, Result, Settings,
    format::DateContext,
    model::*,
    model_actions::Action,
    model_dialogs::{DialogInfo, DialogsInfo},
    model_media::MediaContent,
    model_message::MessagesSlice,
    saved::*,
    saved_parse::parse_saved_messages,
    settings::Environment,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
const BASE: i64 = 1_700_000_000;
fn users() -> (User, User) {
    (
        User {
            id: 1,
            is_self: true,
            first_name: "Me".into(),
            ..Default::default()
        },
        User {
            id: 100,
            first_name: "Peer".into(),
            access_hash: Some(7),
            ..Default::default()
        },
    )
}
fn message(id: i64, date: i64) -> Value {
    json!({"v":1,"type":"message","id":id,"date":date,"out":false,"text":"","entities":[],"media":null})
}
fn options(path: &Path) -> Settings {
    Settings {
        path: path.into(),
        time_zone: Some("UTC".into()),
        ..Default::default()
    }
}
#[derive(Default)]
struct Recorder {
    calls: Vec<&'static str>,
    slices: Vec<MessagesSlice>,
    settings: Settings,
    dialog: DialogInfo,
}
impl ExportWriter for Recorder {
    fn format(&self) -> Format {
        Format::Html
    }
    fn start(&mut self, s: &Settings, _: &Environment) -> Result<()> {
        self.calls.push("start");
        self.settings = s.clone();
        Ok(())
    }
    fn write_dialogs_start(&mut self, _: &DialogsInfo) -> Result<()> {
        self.calls.push("dialogs_start");
        Ok(())
    }
    fn write_dialog_start(&mut self, d: &DialogInfo) -> Result<()> {
        self.calls.push("dialog_start");
        self.dialog = d.clone();
        Ok(())
    }
    fn write_dialog_slice(&mut self, s: &MessagesSlice) -> Result<()> {
        self.calls.push("slice");
        self.slices.push(s.clone());
        Ok(())
    }
    fn write_dialog_end(&mut self) -> Result<()> {
        self.calls.push("dialog_end");
        Ok(())
    }
    fn write_dialogs_end(&mut self) -> Result<()> {
        self.calls.push("dialogs_end");
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        self.calls.push("finish");
        Ok(())
    }
    fn main_file_path(&self) -> PathBuf {
        self.settings.path.clone()
    }
}
#[test]
fn records_ids_replies_actions() {
    let (own, peer) = users();
    let mut first = message(-55, BASE);
    first["out"] = json!(true);
    first["text"] = json!("hi bold");
    first["entities"] = json!([{"_":"messageEntityCustomEmoji","offset":0,"length":2,"document_id":9},{"_":"messageEntityBold","offset":3,"length":4},{"_":"somethingElse","offset":0,"length":1}]);
    let mut reply = message(77, BASE + 3);
    reply["reply_to"] = json!(-55);
    let mut missing = message(78, BASE + 4);
    missing["reply_to"] = json!(12345);
    let service = |i, kind, out| json!({"type":"service","id":format!("service-{i}"),"date":BASE+i,"out":out,"action":{"_":kind,"ttl_seconds":30}});
    let rows = vec![
        first,
        service(1, "decryptedMessageActionSetMessageTTL", false),
        service(2, "decryptedMessageActionDeleteMessages", false),
        reply,
        missing,
        service(5, "decryptedMessageActionScreenshotMessages", false),
        service(6, "decryptedMessageActionFlushHistory", true),
    ];
    let m = parse_saved_messages(&rows, &own, &peer, &DateContext::utc()).unwrap();
    assert_eq!(
        m.iter().map(|m| m.id).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        (m[0].from_id, m[0].peer_id, m[0].self_id, m[0].out),
        (1, 100, 1, true)
    );
    assert_eq!(
        m[0].text
            .iter()
            .map(|t| (t.kind, t.text.as_str()))
            .collect::<Vec<_>>(),
        [
            (TextType::CustomEmoji, "hi"),
            (TextType::Text, " "),
            (TextType::Bold, "bold")
        ]
    );
    assert_eq!(m[0].text[0].additional, TextPart::UNAVAILABLE_EMOJI);
    assert!(matches!(&m[1].action.content,Some(Action::SetMessagesTTL(a))if a.period==30));
    assert_eq!(
        (m[2].from_id, m[2].reply_to_msg_id, m[3].reply_to_msg_id),
        (100, 1, 0)
    );
    assert!(matches!(
        m[4].action.content,
        Some(Action::ScreenshotTaken(_))
    ));
    assert!(matches!(m[5].action.content, Some(Action::HistoryClear(_))));
    assert_eq!(m[5].from_id, 1);
}
#[test]
fn saved_media_mapping() {
    let (own, peer) = users();
    let media = vec![
        json!({"_":"decryptedMessageMediaPhoto","w":800,"h":600,"size":50,"caption":"cap"}),
        json!({"_":"decryptedMessageMediaDocument","mime_type":"video/mp4","attributes":[{"_":"documentAttributeVideo","round_message":true,"duration":3,"w":240,"h":240}]}),
        json!({"_":"decryptedMessageMediaDocument","mime_type":"audio/ogg","attributes":[{"_":"documentAttributeAudio","voice":true,"duration":2}]}),
        json!({"_":"decryptedMessageMediaDocument","mime_type":"image/webp","attributes":[{"_":"documentAttributeSticker","alt":"x"}]}),
        json!({"_":"decryptedMessageMediaDocument","mime_type":"video/mp4","attributes":[{"_":"documentAttributeAnimated"},{"_":"documentAttributeFilename","file_name":"a:b.mp4"}]}),
        json!({"_":"decryptedMessageMediaVideo","duration":4,"w":1,"h":1,"size":10}),
        json!({"_":"decryptedMessageMediaGeoPoint","lat":1.5,"long":2.5}),
        json!({"_":"decryptedMessageMediaContact","phone_number":"1","first_name":"A","last_name":"","user_id":5}),
        json!({"_":"decryptedMessageMediaWebPage","url":"https://t.me"}),
    ];
    let rows = media
        .into_iter()
        .enumerate()
        .map(|(i, media)| {
            let mut row = message(i as i64 + 1, BASE);
            row["media"] = media;
            row
        })
        .collect::<Vec<_>>();
    let m = parse_saved_messages(&rows, &own, &peer, &DateContext::utc()).unwrap();
    let Some(MediaContent::Photo(p)) = &m[0].media.content else {
        panic!()
    };
    assert_eq!((p.image.width, p.image.height), (800, 600));
    assert_eq!(
        p.image.file.suggested_path,
        "photos/photo_1@14-11-2023_22-13-20.jpg"
    );
    assert_eq!(m[0].text, [TextPart::plain("cap")]);
    assert_eq!(
        m[1..6]
            .iter()
            .map(|m| m.media.file().suggested_path)
            .collect::<Vec<_>>(),
        [
            "round_video_messages/file_1@14-11-2023_22-13-20.mp4",
            "voice_messages/audio_1@14-11-2023_22-13-20.ogg",
            "stickers/file_2@14-11-2023_22-13-20.webp",
            "animations/a_b.mp4",
            "video_files/video_1@14-11-2023_22-13-20.mp4"
        ]
    );
    assert!(matches!(&m[1].media.content,Some(MediaContent::Document(d))if d.is_video_message));
    assert!(matches!(&m[2].media.content,Some(MediaContent::Document(d))if d.duration==2));
    assert!(matches!(&m[6].media.content,Some(MediaContent::GeoPoint(p))if p.valid));
    assert!(matches!(
        &m[7].media.content,
        Some(MediaContent::SharedContact(_))
    ));
    assert!(m[8].media.content.is_none());
}
fn photo_row(id: i64, date: i64, source: Option<&Path>, size: u64) -> Value {
    let mut row = message(id, date);
    row["media"] = json!({"_":"decryptedMessageMediaPhoto","w":1,"h":1,"size":size});
    if let Some(p) = source {
        row["file"] = json!(p);
    }
    row
}
#[test]
fn copies_media_policy_and_collision() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("saved");
    fs::write(&source, b"sssss").unwrap();
    let (own, peer) = users();
    let mut rows = vec![
        photo_row(1, BASE, Some(&source), 5),
        photo_row(2, BASE + 1, None, 5),
    ];
    for id in [3, 4] {
        let mut r = message(id, BASE + id);
        r["media"] = json!({"_":"decryptedMessageMediaDocument","mime_type":"application/pdf","size":5,"attributes":[{"_":"documentAttributeFilename","file_name":"r.pdf"}]});
        r["file"] = json!(source);
        rows.push(r);
    }
    rows.push(photo_row(5, BASE + 4, Some(&source), 9 * 1024 * 1024));
    let mut s = options(&d.path().join("out"));
    s.media.types = MediaSettings::PHOTO | MediaSettings::FILE;
    let mut w = Recorder::default();
    let r = export_saved_chat(&rows, &s, &mut w, &own, &peer, &Environment::default()).unwrap();
    assert_eq!(
        w.calls,
        [
            "start",
            "dialogs_start",
            "dialog_start",
            "slice",
            "dialog_end",
            "dialogs_end",
            "finish"
        ]
    );
    assert_eq!(w.settings.single_peer, Some(100));
    assert_eq!(w.dialog.relative_path, "");
    assert_eq!(w.dialog.r#type, export::model_dialogs::DialogType::Personal);
    assert_eq!((r.messages, r.files, r.takeout), (5, 3, false));
    let files = w.slices[0]
        .list
        .iter()
        .map(|m| m.media.file())
        .collect::<Vec<_>>();
    assert_eq!(
        files[0].relative_path,
        "photos/photo_1@14-11-2023_22-13-20.jpg"
    );
    assert_eq!(files[1].skip_reason, SkipReason::Unavailable);
    assert_eq!(files[2].relative_path, "files/r.pdf");
    assert_eq!(files[3].relative_path, "files/r (1).pdf");
    assert_eq!(files[4].skip_reason, SkipReason::FileSize);
    assert_eq!(fs::read(s.path.join("files/r (1).pdf")).unwrap(), b"sssss");
    assert_eq!(
        w.slices[0].peers.keys().copied().collect::<Vec<_>>(),
        [1, 100]
    );
    s.path = d.path().join("onlyfiles");
    s.media.types = MediaSettings::FILE;
    let mut w = Recorder::default();
    export_saved_chat(&rows[..1], &s, &mut w, &own, &peer, &Environment::default()).unwrap();
    assert_eq!(
        w.slices[0].list[0].media.file().skip_reason,
        SkipReason::FileType
    );
}
#[test]
fn saved_dates_handed_to_writer() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("saved");
    fs::write(&source, b"sssss").unwrap();
    let rows = (1..=3)
        .map(|i| photo_row(i, BASE + i * 100, Some(&source), 5))
        .collect::<Vec<_>>();
    let (own, peer) = users();
    let mut s = options(&d.path().join("out"));
    s.single_peer_from = BASE + 150;
    s.single_peer_till = BASE + 250;
    let mut w = Recorder::default();
    export_saved_chat(&rows, &s, &mut w, &own, &peer, &Environment::default()).unwrap();
    assert_eq!(
        w.slices[0]
            .list
            .iter()
            .map(|m| m.media.file().relative_path)
            .collect::<Vec<_>>(),
        ["", "photos/photo_2@14-11-2023_22-16-40.jpg", ""]
    );
    assert!(
        w.slices[0]
            .list
            .iter()
            .all(|m| m.media.file().skip_reason == SkipReason::None)
    );
    s.single_peer_from = BASE + 1000;
    s.single_peer_till = 0;
    let mut w = Recorder::default();
    let r = export_saved_chat(&rows, &s, &mut w, &own, &peer, &Environment::default()).unwrap();
    assert_eq!((r.messages, r.files), (0, 0));
    assert_eq!(w.dialog.messages_count_per_split, [0]);
    assert!(w.slices.is_empty());
}
fn fixture(d: &Path) -> (Vec<Value>, PathBuf, Vec<u8>) {
    let source = d.join("saved source/7/messages.jsonl");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    let mut a = message(99, BASE);
    a["out"] = json!(true);
    a["text"] = json!("hello فارسی");
    let mut b = message(100, BASE + 100);
    b["text"] = json!("reply");
    b["reply_to"] = json!(99);
    let records = vec![a, b];
    let bytes = records
        .iter()
        .map(|r| format!("{r}\n"))
        .collect::<String>()
        .into_bytes();
    fs::write(&source, &bytes).unwrap();
    (records, source, bytes)
}
fn manager(source: &Path) -> support::Manager {
    let (a, _, _) = support::managers();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        a.start_auto_save_secret_chats(source.parent().unwrap().parent().unwrap())
            .await
            .unwrap();
        a.stop_auto_save_secret_chats().await.unwrap();
    });
    assert!(a.list().unwrap().is_empty());
    a
}
fn offline(format: Format) {
    let d = tempfile::tempdir().unwrap();
    let (_, source, before) = fixture(d.path());
    let (own, peer) = users();
    let mut s = options(Path::new("unused"));
    s.format = format;
    let manager = manager(&source);
    let r = manager
        .export_saved_messages(7, &d.path().join("export destination"), &own, &peer, &s)
        .unwrap();
    assert!(manager.list().unwrap().is_empty());
    assert_eq!((r.messages, r.files, r.takeout), (2, 0, false));
    if matches!(format, Format::Json | Format::Both) {
        let v: Value =
            serde_json::from_slice(&fs::read(r.path.join("result.json")).unwrap()).unwrap();
        assert_eq!(v["messages"][0]["id"], 1);
        assert_eq!(v["messages"][1]["id"], 2);
        assert_eq!(v["messages"][0]["text"], "hello فارسی");
        assert_eq!(v["messages"][1]["reply_to_message_id"], 1);
    }
    if matches!(format, Format::Html | Format::Both) {
        let page = fs::read_to_string(r.path.join("messages.html")).unwrap();
        assert!(page.contains("hello فارسی") && page.contains("message2"));
        assert!(r.path.join("css/style.css").is_file() && r.path.join("js/script.js").is_file());
    }
    assert_eq!(fs::read(source).unwrap(), before);
}
#[test]
fn offline_html() {
    offline(Format::Html)
}
#[test]
fn offline_json() {
    offline(Format::Json)
}
#[test]
fn offline_both() {
    offline(Format::Both)
}
fn invalid(value: Value) {
    let d = tempfile::tempdir().unwrap();
    let (_, source, before) = fixture(d.path());
    assert!(SavedExportOptions::try_from(&value).is_err());
    assert!(!d.path().join("invalid").exists());
    assert_eq!(fs::read(source).unwrap(), before);
}
#[test]
fn invalid_format() {
    invalid(json!({"format":"bad"}))
}
#[test]
fn invalid_negative_size() {
    invalid(json!({"size_limit":-1}))
}
#[test]
fn invalid_boolean_size() {
    invalid(json!({"size_limit":true}))
}
#[test]
fn invalid_negative_date() {
    invalid(json!({"date_from":-1}))
}
#[test]
fn invalid_date_order() {
    invalid(json!({"date_till":2,"date_from":3}))
}
#[test]
fn invalid_media_group() {
    invalid(json!({"media_types":["unknown"]}))
}
#[test]
fn invalid_media_string() {
    invalid(json!({"media_types":"photo"}))
}
#[test]
fn overlap_refusal_preserves_source() {
    let d = tempfile::tempdir().unwrap();
    let (_, source, before) = fixture(d.path());
    let (own, peer) = users();
    let s = options(d.path());
    let manager = manager(&source);
    assert_eq!(
        manager
            .export_saved_messages(7, source.parent().unwrap(), &own, &peer, &s)
            .unwrap_err(),
        ExportError::InvalidOptions
    );
    assert_eq!(fs::read(source).unwrap(), before);
}
#[test]
fn failure_hides_contents_and_preserves_source() {
    let d = tempfile::tempdir().unwrap();
    let (_, source, before) = fixture(d.path());
    let (own, peer) = users();
    let destination = d.path().join("private message and credentials");
    fs::write(&destination, b"blocker").unwrap();
    let mut s = options(d.path());
    s.format = Format::Json;
    let manager = manager(&source);
    let e = manager
        .export_saved_messages(7, &destination, &own, &peer, &s)
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "Saved export failed; saved messages were not changed"
    );
    assert_eq!(fs::read(source).unwrap(), before);
}
#[test]
fn copy_failure_closes_output_handles() {
    let d = tempfile::tempdir().unwrap();
    let (mut rows, source, before) = fixture(d.path());
    let photo = source.parent().unwrap().join("photo.jpg");
    fs::write(&photo, b"synthetic copy failure input").unwrap();
    rows[0] = photo_row(99, BASE, Some(&photo), 28);
    let out = d.path().join("output");
    let s = options(&out);
    let (own, peer) = users();
    {
        let mut w = export::HtmlAndJsonWriter::new(
            Box::new(export::html_writer::HtmlWriter::new()),
            Box::new(export::json_writer::JsonWriter::new()),
        );
        let result = export_saved_chat_with_copier(
            &rows,
            &s,
            &mut w,
            &own,
            &peer,
            &Environment::default(),
            |_, _| Err(ExportError::Filesystem),
        );
        assert_eq!(result.unwrap_err(), ExportError::Filesystem);
        assert!(out.join("result.json").is_file());
    }
    let output = out.join("result.json");
    let moved = out.join("closed.json");
    fs::rename(&output, &moved).unwrap();
    fs::remove_file(moved).unwrap();
    #[cfg(target_os = "linux")]
    for entry in fs::read_dir("/proc/self/fd").unwrap() {
        if let Ok(target) = fs::read_link(entry.unwrap().path()) {
            assert!(!target.starts_with(&out), "leaked export file descriptor");
        }
    }
    assert_eq!(fs::read(photo).unwrap(), b"synthetic copy failure input");
    assert_eq!(fs::read(source).unwrap(), before);
}
#[test]
fn date_range_reaches_native_writer() {
    let d = tempfile::tempdir().unwrap();
    let (_, source, _) = fixture(d.path());
    let (own, peer) = users();
    let mut s = options(d.path());
    s.format = Format::Json;
    s.single_peer_from = BASE + 50;
    s.single_peer_till = BASE + 150;
    let manager = manager(&source);
    let result = manager
        .export_saved_messages(7, &d.path().join("limited"), &own, &peer, &s)
        .unwrap();
    let v: Value =
        serde_json::from_slice(&fs::read(result.path.join("result.json")).unwrap()).unwrap();
    assert_eq!(
        v["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [2]
    );
}
