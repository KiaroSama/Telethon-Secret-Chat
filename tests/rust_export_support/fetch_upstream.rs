//! Telegram Desktop 7.4.4 request sequencing, checked independently of the old paging fake.
use super::*;
use std::collections::VecDeque;

struct Reply {
    request: Value,
    scope: RequestScope,
    result: FetchResult<Value>,
}
struct Script {
    base: Fake,
    replies: Mutex<VecDeque<Reply>>,
}
impl Script {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            base: Fake::new(vec![]),
            replies: Mutex::new(replies.into()),
        }
    }
    fn complete(&self) {
        assert!(self.replies.lock().unwrap().is_empty());
    }
}
impl ExportClient for Script {
    fn request<'a>(
        &'a self,
        scope: RequestScope,
        request: Value,
    ) -> BoxFuture<'a, FetchResult<Value>> {
        Box::pin(async move {
            validate_request(&request);
            if matches!(
                kind(&request),
                "messages.GetHistory"
                    | "messages.Search"
                    | "channels.GetChannels"
                    | "channels.GetFullChannel"
                    | "messages.GetChats"
            ) {
                let expected = self.replies.lock().unwrap().pop_front().unwrap();
                assert_eq!(scope, expected.scope);
                for (field, value) in expected.request.as_object().unwrap() {
                    assert_eq!(request[field], *value, "request field {field}");
                }
                if kind(&request) == "messages.Search" {
                    assert!(request.get("offset_date").is_none());
                    assert_eq!(request["min_date"], 0);
                    assert_eq!(request["max_date"], 0);
                }
                expected.result
            } else {
                self.base.request(scope, request).await
            }
        })
    }
}
fn main_scope() -> RequestScope {
    RequestScope {
        takeout: Some(123),
        range: None,
    }
}
fn scope(range: Value) -> RequestScope {
    RequestScope {
        takeout: Some(123),
        range: Some(range),
    }
}
fn history(peer: Value, offset: i64, date: i64, add: i64, limit: i64) -> Value {
    json!({"_":"messages.GetHistory","peer":peer,"offset_id":offset,"offset_date":date,"add_offset":add,"limit":limit,"max_id":0,"min_id":0,"hash":0})
}
fn reply(request: Value, scope: RequestScope, result: Value) -> Reply {
    Reply {
        request,
        scope,
        result: Ok(result),
    }
}
fn messages(name: &str, count: i64, rows: Vec<Value>, offset: Option<i64>) -> Value {
    let mut value = json!({"_":name,"count":count,"messages":rows,"users":[],"chats":[]});
    if let Some(offset) = offset {
        value["offset_id_offset"] = offset.into();
    }
    value
}
async fn run(client: &Script, settings: &Settings) -> (FetchResult<ExportResult>, Recorder) {
    let mut writer = Recorder::default();
    let result = export_single_chat_with_date(
        client,
        settings,
        &mut writer,
        &Environment::default(),
        Some("2026-09-30"),
    )
    .await;
    (result, writer)
}
pub(super) async fn date_requests_and_counts() {
    search_remains_id_based().await;
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path());
    settings.single_peer_from = BASE + 120;
    let peer = settings.single_peer_input.clone().unwrap();
    let ranged = scope(json!({"_":"MessageRange","min_id":1,"max_id":i32::MAX}));
    let client = Script::new(vec![
        reply(
            history(peer.clone(), 0, 0, 0, 1),
            ranged.clone(),
            messages("MessagesSlice", 250, vec![text(250, BASE + 250)], None),
        ),
        reply(
            history(peer.clone(), 0, BASE + 119, 0, 1),
            ranged.clone(),
            messages("MessagesSlice", 250, vec![text(119, BASE + 119)], Some(131)),
        ),
        reply(
            history(peer.clone(), 0, BASE + 119, -100, 100),
            ranged.clone(),
            messages(
                "MessagesSlice",
                250,
                (120..=219).rev().map(|id| text(id, BASE + id)).collect(),
                None,
            ),
        ),
        reply(
            history(peer, 220, 0, -100, 100),
            ranged.clone(),
            messages(
                "Messages",
                31,
                (220..=250).rev().map(|id| text(id, BASE + id)).collect(),
                None,
            ),
        ),
    ]);
    let (result, writer) = run(&client, &settings).await;
    assert_eq!(result.unwrap().messages, 131);
    assert_eq!(writer.dialog.messages_count_per_split, [131]);
    assert_eq!(
        writer.messages().iter().map(|m| m.id).collect::<Vec<_>>(),
        (120..=250).collect::<Vec<_>>()
    );
    assert_eq!(
        writer
            .slices
            .iter()
            .map(|s| s.list.len())
            .collect::<Vec<_>>(),
        [100, 31]
    );
    client.complete();

    // Optional count metadata is authoritative only for the two slice variants.
    for (name, offset, expected) in [
        ("MessagesSlice", None, 3),
        ("ChannelMessages", Some(2), 2),
        ("Messages", Some(1), 3),
    ] {
        let peer = settings.single_peer_input.clone().unwrap();
        let mut options = settings.clone();
        options.path = dir.path().join(name);
        options.single_peer_from = BASE + 1;
        let client = Script::new(vec![
            reply(
                history(peer.clone(), 0, 0, 0, 1),
                ranged.clone(),
                messages("MessagesSlice", 3, vec![text(3, BASE + 3)], None),
            ),
            reply(
                history(peer.clone(), 0, BASE, 0, 1),
                ranged.clone(),
                messages(name, 3, vec![text(1, BASE)], offset),
            ),
            reply(
                history(peer, 0, BASE, -100, 100),
                ranged.clone(),
                messages(
                    "Messages",
                    3,
                    (1..=3).rev().map(|id| text(id, BASE + id)).collect(),
                    None,
                ),
            ),
        ]);
        let (result, writer) = run(&client, &options).await;
        result.unwrap();
        assert_eq!(writer.dialog.messages_count_per_split, [expected]);
        client.complete();
    }
}

async fn search_remains_id_based() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = settings(dir.path());
    options.single_peer = Some(peer_from_channel(300));
    options.single_peer_input = Some(channel());
    options.single_peer_from = BASE;
    let ranged = scope(json!({"_":"MessageRange","min_id":1,"max_id":i32::MAX}));
    let client = Script::new(vec![
        reply(
            json!({"_":"channels.GetChannels"}),
            main_scope(),
            json!({"_":"Chats","chats":[{"_":"Channel","id":300,"title":"Public","broadcast":true,"username":"public_fixture","access_hash":9}]}),
        ),
        Reply {
            request: history(channel(), 0, 0, 0, 1),
            scope: ranged.clone(),
            result: Err(rpc("CHANNEL_PRIVATE", 400, "RPCError")),
        },
        reply(
            json!({"_":"messages.Search","peer":channel(),"from_id":{"_":"InputPeerSelf"},"offset_id":0,"add_offset":0,"limit":1}),
            ranged.clone(),
            messages("MessagesSlice", 3, vec![text(3, BASE + 3)], None),
        ),
        reply(
            json!({"_":"messages.Search","peer":channel(),"from_id":{"_":"InputPeerSelf"},"offset_id":1,"add_offset":-100,"limit":100}),
            ranged,
            messages(
                "Messages",
                3,
                (1..=3).rev().map(|id| text(id, BASE + id)).collect(),
                None,
            ),
        ),
    ]);
    let (result, writer) = run(&client, &options).await;
    result.unwrap();
    client.complete();
    assert!(writer.dialog.only_my_messages);
    assert_eq!(writer.dialog.messages_count_per_split, [3]);
    assert_eq!(
        writer.messages().iter().map(|m| m.id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

fn channel() -> Value {
    json!({"_":"InputPeerChannel","channel_id":300,"access_hash":9})
}
fn channel_info() -> Reply {
    reply(
        json!({"_":"channels.GetChannels"}),
        main_scope(),
        json!({"_":"Chats","chats":[{"_":"Channel","id":300,"title":"SG","megagroup":true,"access_hash":9}]}),
    )
}
fn full() -> Reply {
    reply(
        json!({"_":"channels.GetFullChannel","channel":{"_":"InputChannel","channel_id":300,"access_hash":9}}),
        main_scope(),
        json!({"_":"ChatFull","full_chat":{},"chats":[]}),
    )
}
pub(super) async fn migration_history_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path());
    settings.single_peer = Some(peer_from_channel(300));
    settings.single_peer_input = Some(channel());
    let ranges = (0..3)
        .map(|i| json!({"_":"MessageRange","min_id":i*10+1,"max_id":i*10+10}))
        .collect::<Vec<_>>();
    let migration = json!({"_":"MessageService","id":1,"peer_id":{"_":"PeerChannel","channel_id":300},"date":BASE,"action":{"_":"MessageActionChannelMigrateFrom","title":"Old","chat_id":200}});
    let old = json!({"_":"Chat","id":200,"title":"Old","migrated_to":{"_":"InputChannel","channel_id":300,"access_hash":9}});
    let mut old_message = text(1, BASE + 1);
    old_message["peer_id"] = json!({"_":"PeerChat","chat_id":200});
    let mut new_message = text(2, BASE + 2);
    new_message["peer_id"] = json!({"_":"PeerChannel","channel_id":300});
    let mut replies = vec![
        channel_info(),
        full(),
        reply(
            history(channel(), 1, 0, -1, 1),
            scope(ranges[2].clone()),
            messages("Messages", 1, vec![migration.clone()], None),
        ),
        reply(
            json!({"_":"messages.GetChats","id":[200]}),
            main_scope(),
            json!({"_":"Chats","chats":[old.clone()]}),
        ),
    ];
    for (peer, index, rows) in [
        (json!({"_":"InputPeerChat","chat_id":200}), 0, vec![]),
        (json!({"_":"InputPeerChat","chat_id":200}), 1, vec![]),
        (
            json!({"_":"InputPeerChat","chat_id":200}),
            2,
            vec![old_message.clone()],
        ),
        (channel(), 2, vec![new_message.clone()]),
    ] {
        replies.push(reply(
            history(peer, 0, 0, 0, 1),
            scope(ranges[index].clone()),
            messages("MessagesSlice", rows.len() as i64, rows, None),
        ));
    }
    replies.push(reply(
        history(json!({"_":"InputPeerChat","chat_id":200}), 1, 0, -100, 100),
        scope(ranges[2].clone()),
        messages("Messages", 1, vec![old_message], None),
    ));
    replies.push(reply(
        history(channel(), 1, 0, -100, 100),
        scope(ranges[2].clone()),
        messages("Messages", 1, vec![new_message], None),
    ));
    let mut client = Script::new(replies);
    client.base.splits = ranges.clone();
    let (result, writer) = run(&client, &settings).await;
    result.unwrap();
    client.complete();
    assert_eq!(writer.dialog.splits, [-3, -2, -1, 2]);
    assert_eq!(
        writer.dialog.migrated_from_input,
        json!({"_":"InputPeerChat","chat_id":200})
    );
    assert_eq!(
        writer.messages().iter().map(|m| m.id).collect::<Vec<_>>(),
        [1 + MIGRATED_MESSAGES_ID_SHIFT, 2]
    );

    // Missing actions, mismatched destination metadata, and RPC refusal must
    // leave the current channel export available without attaching another chat.
    for scenario in [
        "no-service",
        "not-modified",
        "wrong-destination",
        "history-rpc",
        "chats-rpc",
    ] {
        let probe = history(channel(), 1, 0, -1, 1);
        let mut replies = vec![channel_info(), full()];
        let response = match scenario {
            "history-rpc" => Err(rpc("CHANNEL_PRIVATE", 400, "RPCError")),
            "not-modified" => Ok(json!({"_":"MessagesNotModified","count":0})),
            "no-service" => {
                let mut ordinary = text(1, BASE);
                ordinary["action"] = migration["action"].clone();
                Ok(messages("Messages", 1, vec![ordinary], None))
            }
            _ => Ok(messages("Messages", 1, vec![migration.clone()], None)),
        };
        replies.push(Reply {
            request: probe,
            scope: scope(ranges[2].clone()),
            result: response,
        });
        if matches!(scenario, "wrong-destination" | "chats-rpc") {
            let mut wrong = old.clone();
            wrong["migrated_to"]["channel_id"] = 999.into();
            replies.push(Reply {
                request: json!({"_":"messages.GetChats","id":[200]}),
                scope: main_scope(),
                result: if scenario == "chats-rpc" {
                    Err(rpc("CHAT_ID_INVALID", 400, "RPCError"))
                } else {
                    Ok(json!({"_":"Chats","chats":[wrong]}))
                },
            });
        }
        replies.push(reply(
            history(channel(), 0, 0, 0, 1),
            scope(ranges[2].clone()),
            messages("Messages", 0, vec![], None),
        ));
        let mut client = Script::new(replies);
        client.base.splits = ranges.clone();
        let mut options = settings.clone();
        options.path = dir.path().join(scenario);
        let (result, writer) = run(&client, &options).await;
        result.unwrap();
        client.complete();
        assert_eq!(writer.dialog.splits, [2]);
        assert!(writer.messages().is_empty());
        assert_eq!(
            writer.dialog.migrated_from_input,
            json!({"_":"InputPeerEmpty"})
        );
    }
    for failing in ["messages.GetHistory", "messages.GetChats"] {
        let mut replies = vec![channel_info(), full()];
        let probe = history(channel(), 1, 0, -1, 1);
        if failing == "messages.GetChats" {
            replies.push(reply(
                probe.clone(),
                scope(ranges[2].clone()),
                messages("Messages", 1, vec![migration.clone()], None),
            ));
        }
        replies.push(Reply {
            request: if failing == "messages.GetHistory" {
                probe
            } else {
                json!({"_":"messages.GetChats","id":[200]})
            },
            scope: if failing == "messages.GetHistory" {
                scope(ranges[2].clone())
            } else {
                main_scope()
            },
            result: Err(export::ExportError::Filesystem.into()),
        });
        let mut client = Script::new(replies);
        client.base.splits = ranges.clone();
        let mut options = settings.clone();
        options.path = dir.path().join(failing);
        let (result, writer) = run(&client, &options).await;
        assert!(matches!(
            result,
            Err(FetchError::Local(export::ExportError::Filesystem))
        ));
        assert!(writer.slices.is_empty());
        client.complete();
        assert_eq!(
            client.base.calls.lock().unwrap().last().unwrap().1["success"],
            false
        );
    }
}
