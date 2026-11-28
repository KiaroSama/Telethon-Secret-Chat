use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use kiaro_secret_chat::{
    autosave::{AutoSaveStore, SavedSecretMessage},
    custody::{CustodyBackend, CustodyMode, CustodyRecord, StoreCustodyLimits},
    errors::SecretChatError,
    export_archive::{ExportFormat, ExportMediaFilter, ExportOptions},
    manager::{BoxFuture, SecretTransport},
    media::{ForwardMediaReference, MediaMetadata, SecretMediaKind},
    orchestration::{
        AutoSaveSubscription, BoundedCustodySettlement, MediaBudgets, MediaOrchestrator,
        RichArchiveRecord, RichTextEntity, ServiceArchiveEvent, export_rich_saved_chat,
        render_rich_html,
    },
    transport::{
        DhConfigData, EncryptedChatWire, EncryptedFileRef, EncryptedFileUpload,
        EstablishmentTransport,
    },
};
use tempfile::tempdir;

type Rpc<'a, T> = BoxFuture<'a, Result<T, SecretChatError>>;
const CHAT: i32 = -41;

fn unexpected_rpc<T>() -> Rpc<'static, T> {
    Box::pin(async { Err(SecretChatError::Protocol("unexpected test RPC")) })
}

type SentFrame = (i32, i64, i64, Vec<u8>);

#[derive(Default)]
struct MediaTransport {
    calls: Mutex<Vec<&'static str>>,
    ciphertext: Mutex<Vec<u8>>,
    upload: Mutex<Option<EncryptedFileUpload>>,
    forward: Mutex<Option<EncryptedFileRef>>,
    sent: Mutex<Vec<SentFrame>>,
}

impl SecretTransport for MediaTransport {
    fn auth_identity(&self) -> &str {
        "test-media-account"
    }

    fn send_encrypted(&self, _: i64, _: i64, _: Vec<u8>) -> Rpc<'_, ()> {
        unexpected_rpc()
    }

    fn discard_encryption(&self, _: i64, _: bool) -> Rpc<'_, ()> {
        unexpected_rpc()
    }
}

impl EstablishmentTransport for MediaTransport {
    fn get_dh_config(&self, _: i32, _: i32) -> Rpc<'_, DhConfigData> {
        unexpected_rpc()
    }

    fn request_encryption(&self, _: i64, _: i64, _: i32, _: Vec<u8>) -> Rpc<'_, EncryptedChatWire> {
        unexpected_rpc()
    }

    fn accept_encryption(&self, _: i32, _: i64, _: Vec<u8>, _: i64) -> Rpc<'_, EncryptedChatWire> {
        unexpected_rpc()
    }

    fn discard_encryption(&self, _: i32, _: bool) -> Rpc<'_, bool> {
        unexpected_rpc()
    }

    #[rustfmt::skip]
    fn send_encrypted_file(
        &self, chat: i32, hash: i64, random: i64, frame: Vec<u8>, file: EncryptedFileUpload,
    ) -> Rpc<'_, i32> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("send");
            self.sent.lock().unwrap().push((chat, hash, random, frame));
            *self.upload.lock().unwrap() = Some(file);
            Ok(123)
        })
    }

    #[rustfmt::skip]
    fn send_encrypted_file_reference(
        &self, chat: i32, hash: i64, random: i64, frame: Vec<u8>, file: EncryptedFileRef,
    ) -> Rpc<'_, i32> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("forward");
            self.sent.lock().unwrap().push((chat, hash, random, frame));
            *self.forward.lock().unwrap() = Some(file);
            Ok(124)
        })
    }

    #[rustfmt::skip]
    fn upload_encrypted_bytes(
        &self, ciphertext: Vec<u8>, key_fingerprint: i32,
    ) -> Rpc<'_, EncryptedFileUpload> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("upload");
            *self.ciphertext.lock().unwrap() = ciphertext;
            Ok(EncryptedFileUpload {
                id: 99,
                parts: 1,
                md5_checksum: "0".repeat(32),
                key_fingerprint,
                is_big: false,
            })
        })
    }

    fn download_encrypted_bytes(&self, _: &EncryptedFileRef) -> Rpc<'_, Vec<u8>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("download");
            Ok(self.ciphertext.lock().unwrap().clone())
        })
    }
}

#[tokio::test]
#[rustfmt::skip]
async fn media_round_trip_and_forward_reuse_the_server_reference() {
    let root = tempdir().unwrap();
    let transport = Arc::new(MediaTransport::default());
    let media = MediaOrchestrator::new(Arc::clone(&transport), MediaBudgets::default()).unwrap();
    let plaintext = b"a local media fixture";
    let sent = media.send_encrypted_media_with_frame(CHAT, 42, SecretMediaKind::Document, plaintext, 7,
        |key| Ok(key.fingerprint.to_le_bytes().to_vec())).await.unwrap();
    assert_eq!(sent.date, 123);
    assert_eq!(*transport.upload.lock().unwrap(), Some(sent.upload.clone()));
    assert_ne!(transport.ciphertext.lock().unwrap().as_slice(), plaintext);
    assert!(!format!("{sent:?}").contains("EncryptedMediaKey"));
    let mut file = file_ref(plaintext.len(), sent.key.fingerprint);
    let path = media.download_and_save_media(&file, plaintext.len(), &sent.key,
        root.path(), "media file.bin").await.unwrap();
    assert_eq!(fs::read(path).unwrap(), plaintext);
    let reference = ForwardMediaReference {
        kind: sent.kind, metadata: MediaMetadata::default(), file_id: file.id,
        access_hash: file.access_hash, dc_id: file.dc_id, size_bytes: plaintext.len() as u64,
        key: sent.key.clone(),
    };
    let (descriptor, date) = media.forward_encrypted_media(
        CHAT + 1, 44, &reference, Some("new caption".into()), vec![8], 9,
    ).await.unwrap();
    assert_eq!(date, 124);
    assert_eq!(descriptor.caption.as_deref(), Some("new caption"));
    assert_eq!(*transport.forward.lock().unwrap(), Some(file.clone()));
    assert_eq!(*transport.calls.lock().unwrap(), ["upload", "send", "download", "forward"]);
    assert_eq!(*transport.sent.lock().unwrap(), [
        (CHAT, 42, 7, sent.key.fingerprint.to_le_bytes().to_vec()), (CHAT + 1, 44, 9, vec![8]),
    ]);
    file.key_fingerprint ^= 1;
    assert!(media.download_and_save_media(&file, plaintext.len(), &sent.key, root.path(), "x")
        .await.is_err());
}

#[rustfmt::skip]
fn file_ref(size: usize, key_fingerprint: i32) -> EncryptedFileRef {
    EncryptedFileRef { id: 100, access_hash: 43, size: size.next_multiple_of(16) as i64,
        dc_id: 2, key_fingerprint }
}

#[tokio::test]
#[rustfmt::skip]
async fn media_budget_refusals_precede_transport_and_reject_oversized_responses() {
    let root = tempdir().unwrap();
    let transport = Arc::new(MediaTransport::default());
    let budgets = MediaBudgets { max_upload_bytes: 17, max_download_bytes: 17 };
    let invalid = MediaBudgets { max_upload_bytes: 0, ..budgets };
    assert!(MediaOrchestrator::new(Arc::clone(&transport), invalid).is_err());
    let media = MediaOrchestrator::new(Arc::clone(&transport), budgets).unwrap();
    let send = |bytes: &'static [u8]| media.send_encrypted_media(CHAT, 42, SecretMediaKind::Document, bytes, vec![1], 7);
    assert!(send(&[0; 17]).await.is_err());
    assert!(transport.calls.lock().unwrap().is_empty());
    let sent = send(&[0; 16]).await.unwrap();
    transport.calls.lock().unwrap().clear();
    let mut file = file_ref(17, sent.key.fingerprint);
    assert!(media.download_and_save_media(&file, 17, &sent.key, root.path(), "x").await.is_err());
    assert!(transport.calls.lock().unwrap().is_empty());
    file.size = 16;
    transport.ciphertext.lock().unwrap().push(0);
    assert!(media.download_and_save_media(&file, 16, &sent.key, root.path(), "x").await.is_err());
    assert_eq!(*transport.calls.lock().unwrap(), ["download"]);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

fn message(id: i64, text: &str) -> SavedSecretMessage {
    SavedSecretMessage {
        id,
        chat_id: CHAT,
        sender_id: 1,
        sender_name: "Test <sender>".into(),
        outgoing: id % 2 == 0,
        date_unix: 1_700_000_000 + id,
        text: text.into(),
        reply_to_msg_id: None,
        ttl_seconds: None,
        media: None,
    }
}

#[tokio::test]
async fn autosave_requires_opt_in_for_both_directions_and_keeps_prior_history() {
    let root = tempdir().unwrap();
    let store = AutoSaveStore::open(root.path().join("history.bin"), [42; 32]).unwrap();
    let sub = AutoSaveSubscription::new(store);
    let incoming = message(1, "incoming");
    let outgoing = message(2, "outgoing");
    assert!(!sub.is_subscribed(CHAT).await);
    assert!(!sub.on_message(incoming.clone()).await.unwrap());
    sub.subscribe_chat(CHAT).await.unwrap();
    assert!(sub.is_subscribed(CHAT).await);
    assert!(sub.on_message(incoming.clone()).await.unwrap());
    assert!(sub.on_message(outgoing.clone()).await.unwrap());
    assert!(!sub.on_message(outgoing.clone()).await.unwrap());
    sub.unsubscribe_chat(CHAT).await.unwrap();
    assert!(!sub.is_subscribed(CHAT).await);
    assert!(!sub.on_message(message(3, "ignored")).await.unwrap());
    assert_eq!(
        sub.read_history(CHAT, 1).await,
        std::slice::from_ref(&outgoing)
    );
    assert_eq!(sub.read_history(CHAT, 10).await, [outgoing, incoming]);
}

#[tokio::test]
async fn autosave_write_error_leaves_prior_history_readable() {
    let root = tempdir().unwrap();
    let path = root.path().join("history.bin");
    let sub = AutoSaveSubscription::new(AutoSaveStore::open(path.clone(), [42; 32]).unwrap());
    sub.subscribe_chat(CHAT).await.unwrap();
    let prior = message(1, "prior");
    assert!(sub.on_message(prior.clone()).await.unwrap());
    fs::rename(&path, root.path().join("history.backup")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(sub.on_message(message(2, "not written")).await.is_err());
    assert_eq!(sub.read_history(CHAT, 10).await, [prior]);
}

#[test]
#[rustfmt::skip]
fn rich_spans_render_only_allowed_markup_and_valid_utf8_ranges() {
    use RichTextEntity::*;
    let text = "é I C L S P M T <script>";
    let entities = [
        Spoiler { offset: 9, len: 1 }, Bold { offset: 0, len: 2 }, Italic { offset: 3, len: 1 },
        Code { offset: 5, len: 1 }, Pre { offset: 11, len: 1, language: Some("<lang>".into()) },
        TextUrl { offset: 7, len: 1, url: "https://example.test/?x=1&y=2".into() },
        Mention { offset: 13, len: 1 }, Strikethrough { offset: 15, len: 1 },
    ];
    let html = render_rich_html(text, &entities).unwrap();
    for fragment in [
        "<strong>é</strong>", "<em>I</em>", "<code>C</code>",
        "href=\"https://example.test/?x=1&amp;y=2\"", "<span class=\"spoiler\">S</span>", "<pre>",
        "&lt;lang&gt;", "<span class=\"mention\">M</span>", "<s>T</s>", "&lt;script&gt;",
    ] {
        assert!(html.contains(fragment), "missing {fragment}");
    }
    assert!(!html.contains("<script>"));
    for bad in [
        vec![Bold { offset: 1, len: 1 }],
        vec![Code { offset: usize::MAX, len: 1 }],
        vec![Bold { offset: 0, len: 2 }, Italic { offset: 0, len: 2 }],
        vec![Code { offset: 0, len: 0 }],
        vec![TextUrl { offset: 3, len: 1, url: "javascript:alert(1)".into() }],
    ] {
        assert!(render_rich_html(text, &bad).is_err());
    }
}

fn options(output_dir: PathBuf) -> ExportOptions {
    ExportOptions {
        chat_id: CHAT,
        chat_title: "Private <chat>".into(),
        peer_id: 1,
        format: ExportFormat::HtmlAndJson,
        output_dir,
        assets_dir: None,
        media_root: None,
        media_filter: ExportMediaFilter {
            photos: false,
            videos: false,
            voice_messages: false,
            video_messages: false,
            stickers: false,
            gifs: false,
            files: false,
            size_limit_bytes: 1,
        },
        date_from_unix: Some(message(1, "").date_unix),
        date_to_unix: None,
        messages_per_page: 2,
    }
}

fn rich(id: i64, text: &str, service_event: Option<ServiceArchiveEvent>) -> RichArchiveRecord {
    RichArchiveRecord {
        base: message(id, text),
        entities: vec![],
        service_event,
    }
}

#[test]
#[rustfmt::skip]
fn rich_export_preserves_base_report_and_renders_service_entries() {
    let root = tempdir().unwrap();
    let output = root.path().join("rich archive");
    fs::create_dir(&output).unwrap();
    use ServiceArchiveEvent::*;
    let mut regular = rich(5, "bold <script>", None);
    regular.entities.push(RichTextEntity::Bold { offset: 0, len: 4 });
    let records = [regular, rich(4, "", Some(HistoryCleared)), rich(3, "", Some(ScreenshotTaken)),
        rich(2, "", Some(TtlChanged { seconds: 30 })),
        rich(1, "", Some(ChatCreated { title: "<new>".into() })), rich(0, "outside date range", None),
    ];
    let report = export_rich_saved_chat(&records, &options(output.clone())).unwrap();
    assert_eq!(report.output_dir, output);
    assert_eq!(report.exported_messages, 5);
    assert!(report.html_pages.iter().all(|path| path.is_file()));
    assert_eq!(report.json_path, Some(output.join("result.json")));
    let html = fs::read_to_string(output.join("rich_messages.html")).unwrap();
    for text in [
        "Chat created: &lt;new&gt;", "TTL changed to 30 seconds", "Screenshot taken",
        "History cleared", "<strong>bold</strong> &lt;script&gt;",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(!html.contains("outside date range"));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("rich_messages.json")).unwrap()).unwrap();
    let entries = json["messages"].as_array().unwrap();
    assert_eq!(entries.len(), 5);
    assert_eq!(entries[1]["type"], "service");
    assert_eq!(entries[1]["action"], "ttl_changed");
    assert_eq!(entries[4]["text_entities"][0]["type"], "bold");
}

#[test]
#[rustfmt::skip]
fn failed_rich_export_does_not_publish_or_leave_staging() {
    let root = tempdir().unwrap();
    let mut invalid = rich(2, "url", None);
    invalid.entities.push(RichTextEntity::TextUrl { offset: 0, len: 3, url: "javascript:alert(1)".into() });
    let opts = options(root.path().join("unpublished"));
    let invalid_ttl = rich(1, "", Some(ServiceArchiveEvent::TtlChanged { seconds: -1 }));
    let duplicate = rich(1, "duplicate", None);
    assert!(export_rich_saved_chat(&[rich(1, "valid first", None), invalid], &opts).is_err());
    assert!(export_rich_saved_chat(&[invalid_ttl], &opts).is_err());
    assert!(export_rich_saved_chat(&[duplicate.clone(), duplicate], &opts).is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
#[rustfmt::skip]
fn custody_pause_preserves_obligations_until_consumer_settles_them() {
    let limits = StoreCustodyLimits { max_retained_messages: 2, max_stored_bytes: 5 };
    let mut custody = BoundedCustodySettlement::new(limits).unwrap();
    custody.record_obligation(CHAT, 7, vec![7; 2]).unwrap();
    custody.record_obligation(CHAT, 1, vec![1; 3]).unwrap();
    custody.record_obligation(CHAT, 1, vec![1; 3]).unwrap();
    assert!(custody.record_obligation(CHAT, 1, vec![2; 3]).is_err());
    assert!(custody.record_obligation(CHAT, 9, vec![9]).is_err());
    let status = custody.settlement_status(CHAT);
    assert!(status.paused);
    assert!(status.pause_reason.is_some());
    assert_eq!((status.unsettled_count, status.unsettled_bytes), (2, 5));
    assert_eq!(custody.poll_unsettled(CHAT, 0, 1).unwrap()[0].sequence_id, 1);
    let second = custody.poll_unsettled(CHAT, 1, 1).unwrap();
    assert_eq!((second[0].sequence_id, second[0].payload_bytes), (7, 2));
    assert_eq!(custody.poll_unsettled(CHAT, 1, 1).unwrap(), second);
    assert!(custody.poll_unsettled(CHAT, 0, 0).is_err());
    assert!(custody.poll_unsettled(CHAT, 0, 101).is_err());
    assert!(custody.resume_if_under_limits(CHAT).is_err());
    assert!(custody.settle_delivery(CHAT, 1).unwrap());
    assert!(!custody.settle_delivery(CHAT, 1).unwrap());
    custody.resume_if_under_limits(CHAT).unwrap();
    assert!(!custody.settlement_status(CHAT).paused);
    assert_eq!(custody.settlement_status(CHAT).unsettled_bytes, 2);
    custody.record_obligation(CHAT, 9, vec![9; 3]).unwrap();
}

#[test]
fn manager_missed_ingress_cannot_be_reversed_by_clearing_local_obligations() {
    let limits = StoreCustodyLimits::default();
    let mut custody = BoundedCustodySettlement::new(limits).unwrap();
    custody.record_obligation(CHAT, 1, vec![1]).unwrap();
    let mut manager_status = CustodyRecord::new(
        i64::from(CHAT),
        "test",
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        limits,
    );
    manager_status.pause("required ingress was missed while custody was paused");
    custody.observe_manager_status(&manager_status).unwrap();
    assert!(custody.settlement_status(CHAT).paused);
    assert!(custody.record_obligation(CHAT, 2, vec![2]).is_err());
    assert!(custody.settle_delivery(CHAT, 1).unwrap());
    manager_status.resume();
    custody.observe_manager_status(&manager_status).unwrap();
    assert!(custody.resume_if_under_limits(CHAT).is_err());
    assert!(custody.settlement_status(CHAT).paused);
    custody.record_obligation(CHAT + 1, 1, vec![1]).unwrap();
}

#[tokio::test]
async fn lifecycle_subscription_retains_only_the_events_selected_by_its_policy() {
    use kiaro_secret_chat::orchestration::{
        BackpressurePolicy::{DropNewest, DropOldest, Refuse},
        BoundedLifecycleSubscription, LifecycleEvent, LifecyclePublishError,
    };
    use std::time::Duration;

    assert!(BoundedLifecycleSubscription::new(0, DropOldest).is_err());
    let first = LifecycleEvent::ChatPaused { chat_id: 7 };
    let second = LifecycleEvent::ChatResumed { chat_id: 7 };
    for (policy, expected, drops) in [
        (DropOldest, second, 1),
        (DropNewest, first, 1),
        (Refuse, first, 0),
    ] {
        let subscription = BoundedLifecycleSubscription::new(1, policy).unwrap();
        subscription.publish(first).unwrap();
        let result = subscription.publish(second);
        if policy == Refuse {
            assert_eq!(result, Err(LifecyclePublishError::Full(second)));
        } else {
            assert_eq!(result, Ok(()));
        }
        assert_eq!(subscription.dropped_count(), drops);
        subscription.close();
        assert_eq!(
            subscription.publish(second),
            Err(LifecyclePublishError::Closed(second))
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            assert_eq!(subscription.recv().await, Some(expected));
            assert_eq!(subscription.recv().await, None);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn lifecycle_subscription_wakes_a_receiver_and_unsubscribe_discards_backlog() {
    use kiaro_secret_chat::orchestration::{
        BackpressurePolicy, BoundedLifecycleSubscription, LifecycleEvent,
    };
    use std::{
        future::{Future, poll_fn},
        task::Poll,
        time::Duration,
    };

    let subscription = BoundedLifecycleSubscription::new(2, BackpressurePolicy::Refuse).unwrap();
    let event = LifecycleEvent::ChatClosed { chat_id: 7 };
    let mut receive = std::pin::pin!(subscription.recv());
    poll_fn(|cx| {
        assert!(receive.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    subscription.publish(event).unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), receive)
            .await
            .unwrap(),
        Some(event)
    );
    assert!(
        subscription
            .publish_manager_event(&kiaro_secret_chat::manager::Event::ChatClosed {
                chat_id: 7,
                reason: "fixture",
                history_deleted: false,
            })
            .unwrap()
    );
    assert!(
        !subscription
            .publish_manager_event(&kiaro_secret_chat::manager::Event::MessageAcknowledged {
                chat_id: 7,
                seq_no: 0,
                random_ids: vec![5],
            })
            .unwrap()
    );
    subscription.unsubscribe();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), subscription.recv())
            .await
            .unwrap(),
        None
    );
}
