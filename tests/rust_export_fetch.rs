#![allow(clippy::unwrap_used)]
#[path = "rust_export_support/tl_contract.rs"]
mod tl_contract;
#[path = "rust_export_support/fetch_upstream.rs"]
mod upstream;
use kiaro_secret_chat::export::{
    self,
    fetch::*,
    fetch_client::*,
    fetch_files::LazyFile,
    model::*,
    model_dialogs::{DialogInfo, DialogsInfo},
    model_message::{Message, MessagesSlice},
    settings::{Environment, Format, MediaSettings, Settings},
    writer::{ExportResult, ExportWriter},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
const BASE: i64 = 1700000000;
const DAY: i64 = 86400;
struct Fake {
    history: Vec<Value>,
    takeout_error: Option<FetchError>,
    splits: Vec<Value>,
    calls: Mutex<Vec<(RequestScope, Value)>>,
    downloads: Mutex<Vec<FileLocation>>,
    download_errors: Mutex<BTreeMap<i64, FetchError>>,
    emoji: Vec<Value>,
    refetched: BTreeMap<i64, Value>,
    migrated: bool,
}
impl Fake {
    fn new(mut history: Vec<Value>) -> Self {
        history.sort_by_key(|m| -integer(m, "id"));
        Self {
            history,
            takeout_error: None,
            splits: vec![json!({"_":"MessageRange","min_id":1,"max_id":i32::MAX})],
            calls: Mutex::new(vec![]),
            downloads: Mutex::new(vec![]),
            download_errors: Mutex::new(BTreeMap::new()),
            emoji: vec![],
            refetched: BTreeMap::new(),
            migrated: false,
        }
    }
}
fn validate_request(request: &Value) {
    macro_rules! check {
        ($ty:ty) => {{
            let _: $ty = serde_json::from_value(export::tl_value::request_args(request)).unwrap();
        }};
    }
    use grammers_tl_types::functions as f;
    match kind(request) {
        "users.GetUsers" => {
            check!(f::users::GetUsers)
        }
        "help.GetCountriesList" => {
            check!(f::help::GetCountriesList)
        }
        "account.InitTakeoutSession" => {
            check!(f::account::InitTakeoutSession)
        }
        "account.FinishTakeoutSession" => {
            check!(f::account::FinishTakeoutSession)
        }
        "messages.GetSplitRanges" => {
            check!(f::messages::GetSplitRanges)
        }
        "messages.GetHistory" => {
            check!(f::messages::GetHistory)
        }
        "messages.Search" => {
            check!(f::messages::Search)
        }
        "messages.GetCustomEmojiDocuments" => {
            check!(f::messages::GetCustomEmojiDocuments)
        }
        "messages.GetMessages" => {
            check!(f::messages::GetMessages)
        }
        "messages.GetChats" => {
            check!(f::messages::GetChats)
        }
        "channels.GetChannels" => {
            check!(f::channels::GetChannels)
        }
        "channels.GetFullChannel" => {
            check!(f::channels::GetFullChannel)
        }
        _ => panic!("unvalidated native request"),
    }
}
impl ExportClient for Fake {
    fn request<'a>(
        &'a self,
        scope: RequestScope,
        request: Value,
    ) -> BoxFuture<'a, FetchResult<Value>> {
        Box::pin(async move {
            validate_request(&request);
            self.calls.lock().unwrap().push((scope, request.clone()));
            Ok(match kind(&request) {
                "users.GetUsers" => {
                    if request["id"][0]["_"] == "InputUserSelf" {
                        json!([{"_":"User","id":1,"is_self":true,"first_name":"Me"}])
                    } else {
                        json!([{"_":"User","id":100,"first_name":"Peer","access_hash":5}])
                    }
                }
                "help.GetCountriesList" => {
                    return Err(rpc("COUNTRIES_UNAVAILABLE", 400, "RPCError"));
                }
                "account.InitTakeoutSession" => {
                    if let Some(e) = &self.takeout_error {
                        return Err(e.clone());
                    }
                    json!({"_":"Takeout","id":123})
                }
                "account.FinishTakeoutSession" => json!(true),
                "messages.GetSplitRanges" => json!(self.splits),
                "messages.GetCustomEmojiDocuments" => Value::Array(
                    self.emoji
                        .iter()
                        .filter(|d| {
                            request["document_id"]
                                .as_array()
                                .unwrap()
                                .contains(&d["id"])
                        })
                        .cloned()
                        .collect(),
                ),
                "messages.GetMessages" => {
                    json!({"_":"Messages","messages":[self.refetched[&integer(&request["id"][0],"id")].clone()],"users":[],"chats":[]})
                }
                "channels.GetChannels" => {
                    json!({"_":"Chats","chats":[{"_":"Channel","id":300,"title":"SG","megagroup":true,"access_hash":9}]})
                }
                "channels.GetFullChannel" => {
                    json!({"_":"ChatFull","full_chat":{"migrated_from_chat_id":200},"chats":[{"_":"Chat","id":200,"title":"Old","migrated_to":{"_":"InputChannel","channel_id":300,"access_hash":9}}]})
                }
                "messages.GetHistory" | "messages.Search" => {
                    let mut migrated;
                    let ordered = if self.migrated {
                        let old = request["peer"]["_"] == "InputPeerChat";
                        migrated = (1..=2)
                            .map(|id| {
                                let mut m = text(id, BASE + if old { id } else { 10 + id });
                                m["peer_id"] = if old {
                                    json!({"_":"PeerChat","chat_id":200})
                                } else {
                                    json!({"_":"PeerChannel","channel_id":300})
                                };
                                m
                            })
                            .collect::<Vec<_>>();
                        migrated.reverse();
                        &migrated
                    } else {
                        &self.history
                    };
                    let offset = integer(&request, "offset_id");
                    let offset_date = integer(&request, "offset_date");
                    let index = if offset_date > 0 {
                        ordered
                            .iter()
                            .position(|m| integer(m, "date") <= offset_date)
                            .unwrap_or(ordered.len())
                    } else if offset == 0 {
                        0
                    } else {
                        ordered
                            .iter()
                            .position(|m| integer(m, "id") < offset)
                            .unwrap_or(ordered.len())
                    } as i64;
                    let start = (index + integer(&request, "add_offset")).max(0) as usize;
                    let end = (index + integer(&request, "add_offset") + integer(&request, "limit"))
                        .max(0) as usize;
                    let rows = ordered
                        .get(start.min(ordered.len())..end.min(ordered.len()))
                        .unwrap_or(&[]);
                    let mut result = json!({"_":"MessagesSlice","count":ordered.len(),"messages":rows,"users":[{"_":"User","id":100,"first_name":"Peer"}],"chats":[]});
                    if offset_date > 0 {
                        result["offset_id_offset"] = index.into();
                    }
                    result
                }
                _ => panic!("unexpected scripted request {}", kind(&request)),
            })
        })
    }
    fn download<'a>(
        &'a self,
        l: &'a FileLocation,
        size: u64,
        target: &'a mut LazyFile,
    ) -> BoxFuture<'a, FetchResult<()>> {
        Box::pin(async move {
            self.downloads.lock().unwrap().push(l.clone());
            if let Some(e) = self
                .download_errors
                .lock()
                .unwrap()
                .remove(&integer(&l.data, "id"))
            {
                return Err(e);
            }
            target.write(&vec![b'x'; size.max(1) as usize])?;
            Ok(())
        })
    }
}
fn rpc(name: &str, code: i32, kind: &str) -> FetchError {
    FetchError::Rpc {
        code,
        name: name.into(),
        kind: kind.into(),
    }
}
fn text(id: i64, date: i64) -> Value {
    json!({"_":"Message","id":id,"peer_id":{"_":"PeerUser","user_id":100},"date":date,"message":format!("m{id}")})
}
fn photo(id: i64, date: i64) -> Value {
    let mut m = text(id, date);
    m["media"] = json!({"_":"MessageMediaPhoto","photo":{"_":"Photo","id":id*10,"access_hash":1,"file_reference":b"old","date":date,"sizes":[{"_":"PhotoSize","type":"y","w":10,"h":10,"size":100}],"dc_id":2}});
    m
}
fn document(id: i64, doc: i64, name: Option<&str>, size: u64, video: bool) -> Value {
    let mut m = text(id, BASE);
    let mut attributes = vec![];
    if video {
        attributes.push(json!({"_":"DocumentAttributeVideo","duration":1,"w":1,"h":1}));
    }
    if let Some(name) = name {
        attributes.push(json!({"_":"DocumentAttributeFilename","file_name":name}));
    }
    m["media"] = json!({"_":"MessageMediaDocument","document":{"_":"Document","id":doc,"access_hash":1,"file_reference":b"old","date":BASE,"mime_type":"application/pdf","size":size,"dc_id":2,"attributes":attributes}});
    m
}
#[derive(Default)]
struct Recorder {
    calls: Vec<&'static str>,
    slices: Vec<MessagesSlice>,
    settings: Settings,
    dialog: DialogInfo,
}
impl Recorder {
    fn messages(&self) -> Vec<Message> {
        self.slices.iter().flat_map(|s| s.list.clone()).collect()
    }
}
impl ExportWriter for Recorder {
    fn format(&self) -> Format {
        Format::Html
    }
    fn start(&mut self, s: &Settings, _: &Environment) -> export::Result<()> {
        self.settings = s.clone();
        self.calls.push("start");
        Ok(())
    }
    fn write_dialogs_start(&mut self, _: &DialogsInfo) -> export::Result<()> {
        self.calls.push("dialogs_start");
        Ok(())
    }
    fn write_dialog_start(&mut self, d: &DialogInfo) -> export::Result<()> {
        self.dialog = d.clone();
        self.calls.push("dialog_start");
        Ok(())
    }
    fn write_dialog_slice(&mut self, d: &MessagesSlice) -> export::Result<()> {
        self.slices.push(d.clone());
        self.calls.push("slice");
        Ok(())
    }
    fn write_dialog_end(&mut self) -> export::Result<()> {
        self.calls.push("dialog_end");
        Ok(())
    }
    fn write_dialogs_end(&mut self) -> export::Result<()> {
        self.calls.push("dialogs_end");
        Ok(())
    }
    fn finish(&mut self) -> export::Result<()> {
        self.calls.push("finish");
        Ok(())
    }
    fn main_file_path(&self) -> PathBuf {
        self.settings.path.clone()
    }
}
fn settings(path: &Path) -> Settings {
    Settings {
        path: path.into(),
        single_peer: Some(100),
        single_peer_input: Some(json!({"_":"InputPeerUser","user_id":100,"access_hash":5})),
        time_zone: Some("UTC".into()),
        ..Default::default()
    }
}
async fn run(c: &Fake, s: &Settings) -> (ExportResult, Recorder) {
    let mut writer = Recorder::default();
    let result = export_single_chat_with_date(
        c,
        s,
        &mut writer,
        &Environment::default(),
        Some("2026-09-30"),
    )
    .await
    .unwrap();
    (result, writer)
}
#[tokio::test]
async fn takeout_flags_order_and_writer_sequence() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::new((1..=250).map(|i| text(i, BASE + i)).collect());
    let (r, w) = run(&c, &settings(d.path())).await;
    assert!(r.takeout && r.takeout_error.is_empty());
    let calls = c.calls.lock().unwrap();
    let init = &calls
        .iter()
        .find(|(_, r)| kind(r) == "account.InitTakeoutSession")
        .unwrap()
        .1;
    assert_eq!(
        (
            init["contacts"].clone(),
            init["message_users"].clone(),
            init["message_chats"].clone(),
            init["message_megagroups"].clone(),
            init["message_channels"].clone(),
            init["files"].clone(),
            init["file_max_size"].clone()
        ),
        (
            json!(false),
            json!(true),
            json!(true),
            json!(true),
            json!(true),
            json!(true),
            json!(8 * 1024 * 1024)
        )
    );
    assert_eq!(calls.last().unwrap().1["success"], true);
    assert_eq!(&w.calls[..3], ["start", "dialogs_start", "dialog_start"]);
    assert_eq!(
        &w.calls[w.calls.len() - 3..],
        ["dialog_end", "dialogs_end", "finish"]
    );
    assert_eq!(
        w.messages().iter().map(|m| m.id).collect::<Vec<_>>(),
        (1..=250).collect::<Vec<_>>()
    );
    assert_eq!(
        w.slices.iter().map(|s| s.list.len()).collect::<Vec<_>>(),
        [100, 100, 50]
    );
    assert!(
        calls
            .iter()
            .any(|(s, r)| s.range.is_some() && kind(r) == "messages.GetHistory")
    );
    assert!(
        calls
            .iter()
            .filter(|(s, _)| s.range.is_some())
            .all(|(_, r)| kind(r) == "messages.GetHistory")
    );
    assert_eq!(r.path, d.path());
    assert_eq!(
        (w.dialog.relative_path, w.dialog.name),
        (String::new(), "Peer".into())
    );
    assert_eq!(r.messages, 250);
}
#[tokio::test]
async fn refused_takeout_falls_back() {
    let d = tempfile::tempdir().unwrap();
    let mut c = Fake::new(vec![text(1, BASE)]);
    c.takeout_error = Some(rpc("TAKEOUT_INIT_DELAY", 420, "TakeoutInitDelayError"));
    let (r, w) = run(&c, &settings(d.path())).await;
    assert!(!r.takeout);
    assert_eq!(r.takeout_error, "TakeoutInitDelayError");
    let calls = c.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .all(|(s, _)| s.takeout.is_none() && s.range.is_none())
    );
    assert!(
        calls
            .iter()
            .all(|(_, r)| kind(r) != "account.FinishTakeoutSession")
    );
    assert_eq!(w.messages().iter().map(|m| m.id).collect::<Vec<_>>(), [1]);
}
#[tokio::test]
async fn split_ranges_oldest_first() {
    let d = tempfile::tempdir().unwrap();
    let mut c = Fake::new((1..=3).map(|i| text(i, BASE + i)).collect());
    c.splits = vec![
        json!({"_":"MessageRange","min_id":1,"max_id":2}),
        json!({"_":"MessageRange","min_id":2,"max_id":i32::MAX}),
    ];
    let (_, w) = run(&c, &settings(d.path())).await;
    assert_eq!(w.dialog.splits, [0, 1]);
    let ranges = c
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(s, _)| s.range.clone())
        .collect::<Vec<_>>();
    assert!(ranges.contains(&c.splits[0]) && ranges.contains(&c.splits[1]));
    let split_order = ranges.iter().filter(|r| *r == &c.splits[0]).count();
    assert!(split_order > 0);
}
#[tokio::test]
async fn date_limits_leave_outside_files_alone() {
    upstream::date_requests_and_counts().await;
    let d = tempfile::tempdir().unwrap();
    let c = Fake::new(vec![
        photo(1, BASE),
        photo(2, BASE + DAY),
        photo(3, BASE + 3 * DAY),
    ]);
    let mut s = settings(d.path());
    s.single_peer_from = BASE + DAY / 2;
    s.single_peer_till = BASE + 2 * DAY;
    let (_, w) = run(&c, &s).await;
    let rows = w.messages();
    assert_eq!(rows.iter().map(|m| m.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(w.dialog.messages_count_per_split, [2]);
    assert_eq!(rows[1].file().skip_reason, SkipReason::None);
    assert_eq!(rows[1].file().relative_path, "");
    assert!(
        !d.path()
            .join("photos/photo_1@14-11-2023_22-13-20.jpg")
            .exists()
    );
    assert_eq!(
        rows[0].file().relative_path,
        "photos/photo_1@15-11-2023_22-13-20.jpg"
    );
    assert_eq!(
        c.downloads
            .lock()
            .unwrap()
            .iter()
            .map(|l| integer(&l.data, "id"))
            .collect::<Vec<_>>(),
        [20]
    );
}
#[tokio::test]
async fn range_after_all_messages_writes_no_slices() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::new(vec![text(1, BASE)]);
    let mut s = settings(d.path());
    s.single_peer_from = BASE + DAY;
    let (_, w) = run(&c, &s).await;
    assert_eq!(w.dialog.messages_count_per_split, [0]);
    assert!(w.slices.is_empty());
}
#[tokio::test]
async fn media_limits_names_and_dedupe() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::new(vec![
        document(1, 500, Some("report.pdf"), 100, false),
        document(2, 501, Some("report.pdf"), 100, false),
        document(3, 500, Some("report.pdf"), 100, false),
        document(4, 502, None, 10 * 1024 * 1024, true),
        document(5, 503, None, 100, true),
    ]);
    let mut s = settings(d.path());
    s.media.types = MediaSettings::FILE | MediaSettings::VIDEO;
    let (_, w) = run(&c, &s).await;
    let files = w.messages().iter().map(Message::file).collect::<Vec<_>>();
    assert_eq!(files[0].relative_path, "files/report.pdf");
    assert_eq!(files[1].relative_path, "files/report (1).pdf");
    assert_eq!(files[2].relative_path, "files/report.pdf");
    assert_eq!(files[3].skip_reason, SkipReason::FileSize);
    assert_eq!(files[3].relative_path, "");
    assert_eq!(
        files[4].relative_path,
        "video_files/video_2@14-11-2023_22-13-20.pdf"
    );
    assert_eq!(
        c.downloads
            .lock()
            .unwrap()
            .iter()
            .map(|l| integer(&l.data, "id"))
            .collect::<Vec<_>>(),
        [500, 501, 503]
    );
    assert_eq!(
        fs::read(d.path().join("files/report (1).pdf")).unwrap(),
        vec![b'x'; 100]
    );
}
#[tokio::test]
async fn unselected_file_type_skipped() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::new(vec![document(1, 9, Some("a.pdf"), 100, false)]);
    let (_, w) = run(&c, &settings(d.path())).await;
    assert_eq!(w.messages()[0].file().skip_reason, SkipReason::FileType);
    assert!(c.downloads.lock().unwrap().is_empty());
}
#[tokio::test]
async fn reference_refresh_and_unavailable_file() {
    let d = tempfile::tempdir().unwrap();
    let mut c = Fake::new(vec![photo(1, BASE), photo(2, BASE + 1)]);
    c.download_errors.lock().unwrap().extend([
        (
            10,
            rpc("FILE_REFERENCE_EXPIRED", 400, "FileReferenceExpiredError"),
        ),
        (20, rpc("LOCATION_INVALID", 400, "LocationInvalidError")),
    ]);
    let mut fresh = photo(1, BASE);
    fresh["media"]["photo"]["file_reference"] = json!(b"new");
    c.refetched.insert(1, fresh);
    let (_, w) = run(&c, &settings(d.path())).await;
    let rows = w.messages();
    assert_eq!(
        rows[0].file().relative_path,
        "photos/photo_1@14-11-2023_22-13-20.jpg"
    );
    assert_eq!(rows[1].file().skip_reason, SkipReason::Unavailable);
    assert_eq!(
        c.downloads
            .lock()
            .unwrap()
            .iter()
            .filter(|l| integer(&l.data, "id") == 10)
            .map(|l| l.data["file_reference"].clone())
            .collect::<Vec<_>>(),
        [json!(b"old"), json!(b"new")]
    );
    assert!(
        !d.path()
            .join("photos/photo_2@14-11-2023_22-13-21.jpg")
            .exists()
    );
}
#[tokio::test]
async fn custom_emoji_files_and_placeholders() {
    let d = tempfile::tempdir().unwrap();
    let mut message = text(1, BASE);
    message["message"] = json!("a b");
    message["entities"] = json!([{"_":"MessageEntityCustomEmoji","offset":0,"length":1,"document_id":77},{"_":"MessageEntityCustomEmoji","offset":2,"length":1,"document_id":78}]);
    let mut c = Fake::new(vec![message.clone()]);
    c.emoji = vec![
        json!({"_":"Document","id":77,"access_hash":1,"file_reference":[],"date":BASE,"mime_type":"image/webp","size":10,"dc_id":2,"attributes":[{"_":"DocumentAttributeCustomEmoji","alt":"a"}]}),
    ];
    let mut s = settings(d.path());
    s.media.types = MediaSettings::PHOTO | MediaSettings::STICKER;
    let (_, w) = run(&c, &s).await;
    assert_eq!(
        w.messages()[0]
            .text
            .iter()
            .filter(|p| p.kind == TextType::CustomEmoji)
            .map(|p| p.additional.as_str())
            .collect::<Vec<_>>(),
        ["stickers/file_1.webp", "(unavailable)"]
    );
    let c = Fake::new(vec![message]);
    let (_, w) = run(&c, &settings(&d.path().join("second"))).await;
    assert_eq!(
        w.messages()[0]
            .text
            .iter()
            .filter(|p| !p.additional.is_empty())
            .map(|p| p.additional.as_str())
            .collect::<Vec<_>>(),
        ["(unavailable)", "(unavailable)"]
    );
}
#[tokio::test]
async fn populated_destination_chooses_chat_subfolder() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("keep.txt"), "x").unwrap();
    fs::create_dir(d.path().join("ChatExport_2026-09-30")).unwrap();
    let (r, w) = run(&Fake::new(vec![photo(1, BASE)]), &settings(d.path())).await;
    assert_eq!(r.path, d.path().join("ChatExport_2026-09-30 (1)"));
    assert_eq!(w.settings.path, r.path);
    assert!(r.path.join("photos").is_dir());
}
#[tokio::test]
async fn migrated_group_old_chat_first() {
    upstream::migration_history_fallback().await;
    let d = tempfile::tempdir().unwrap();
    let mut c = Fake::new(vec![]);
    c.takeout_error = Some(export::ExportError::InvalidOptions.into());
    c.migrated = true;
    let mut s = settings(d.path());
    s.single_peer = Some(peer_from_channel(300));
    s.single_peer_input = Some(json!({"_":"InputPeerChannel","channel_id":300,"access_hash":9}));
    let (r, w) = run(&c, &s).await;
    assert_eq!(r.takeout_error, "ValueError");
    assert_eq!(w.dialog.splits, [-1, 0]);
    assert_eq!(
        w.dialog.migrated_from_input,
        json!({"_":"InputPeerChat","chat_id":200})
    );
    assert_eq!(
        w.messages().iter().map(|m| m.id).collect::<Vec<_>>(),
        [
            1 + MIGRATED_MESSAGES_ID_SHIFT,
            2 + MIGRATED_MESSAGES_ID_SHIFT,
            1,
            2
        ]
    );
}
#[test]
fn takeout_always_serializes_ceiling() {
    tl_contract::assert_native_tl_bridge_shapes();
    let s = Settings {
        media: MediaSettings {
            size_limit: export::settings::MAX_FILE_SIZE,
            ..Default::default()
        },
        ..Default::default()
    };
    let value = takeout_options(&s);
    assert_eq!(value["files"], true);
    assert_eq!(value["file_max_size"], export::settings::MAX_FILE_SIZE);
    let raw: grammers_tl_types::functions::account::InitTakeoutSession =
        serde_json::from_value(export::tl_value::request_args(&value)).unwrap();
    assert!(raw.files);
    assert_eq!(
        raw.file_max_size,
        Some(export::settings::MAX_FILE_SIZE as i64)
    );
    use grammers_tl_types::Serializable;
    let bytes = raw.to_bytes();
    assert_eq!(
        i64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap()),
        export::settings::MAX_FILE_SIZE as i64
    );
    let user: grammers_tl_types::enums::InputPeer = serde_json::from_value(
        export::tl_value::request_args(&json!({"_":"InputPeerUser","user_id":100,"access_hash":5})),
    )
    .unwrap();
    assert_eq!(
        export::tl_value::to_value(&user).unwrap(),
        json!({"_":"InputPeerUser","user_id":100,"access_hash":5})
    );
    assert_eq!(
        export::tl_value::to_value(&grammers_tl_types::enums::RichText::TextEmpty).unwrap(),
        json!({"_":"TextEmpty"})
    );
    assert_eq!(
        export::tl_value::to_value(&grammers_tl_types::enums::InlineQueryPeerType::Pm).unwrap(),
        json!({"_":"InlineQueryPeerTypePM"})
    );
    let ttl = grammers_tl_types::types::MessageActionSetMessagesTtl {
        period: 30,
        auto_setting_from: None,
    };
    let ttl = export::tl_value::to_value(&ttl).unwrap();
    assert_eq!(ttl["_"], "MessageActionSetMessagesTTL");
    let parsed = export::parse_actions::parse_service_action(
        &mut ParseMediaContext::default(),
        &ttl,
        "",
        BASE,
        &export::format::DateContext::utc(),
    )
    .unwrap();
    assert!(
        matches!(parsed.content,Some(export::model_actions::Action::SetMessagesTTL(a))if a.period==30)
    );
}
