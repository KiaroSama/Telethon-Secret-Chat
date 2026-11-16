#![allow(clippy::unwrap_used)]
#[path = "rust_export_support/fetch_files.rs"]
mod support;
use kiaro_secret_chat::export::model_rich::*;
use kiaro_secret_chat::export::{
    self,
    model_actions::{Action, ActionChatEditPhoto},
    model_media::{Media, MediaContent, SharedContact},
};
use support::*;
#[test]
fn location_keys() {
    assert_eq!(
        compute_location_key(&location(7, b"old", "m", true)),
        (2 | (2 << 24) | (i64::from(b'm') << 16), 7)
    );
    assert_eq!(
        compute_location_key(&location(9, b"old", "", false)),
        (2 | (6 << 24), 9)
    );
    assert_eq!(
        compute_location_key(&FileLocation {
            dc_id: 4,
            data: json!({"_":"InputTakeoutFileLocation"})
        }),
        (4 | (5 << 24), 0)
    );
}
#[test]
fn cache_empty_locations_and_eviction() {
    let mut c = LoadedFileCache::new(1);
    c.save(&FileLocation::default(), "ignored".into());
    assert!(c.find(&FileLocation::default()).is_none());
    c.save(&location(1, b"a", "y", false), "photos/a.jpg".into());
    assert_eq!(c.find(&location(1, b"b", "y", false)), Some("photos/a.jpg"));
    c.save(&location(2, b"a", "y", false), "photos/b.jpg".into());
    assert!(c.find(&location(1, b"a", "y", false)).is_none());
    assert_eq!(c.find(&location(2, b"a", "y", false)), Some("photos/b.jpg"));
}
#[test]
fn lazy_file_first_write_or_finish() {
    let d = tempfile::tempdir().unwrap();
    let mut empty = LazyFile::new(d.path().join("a/empty.bin"));
    assert!(!d.path().join("a").exists());
    empty.finish().unwrap();
    assert_eq!(fs::read(d.path().join("a/empty.bin")).unwrap(), b"");
    let mut target = LazyFile::new(d.path().join("b/data.bin"));
    assert_eq!(target.write(b"12").unwrap(), 2);
    assert_eq!(target.write(b"3").unwrap(), 1);
    target.finish().unwrap();
    target.close();
    assert_eq!(fs::read(d.path().join("b/data.bin")).unwrap(), b"123");
}
#[test]
fn numeric_and_rpc_names() {
    assert_eq!(
        (to_ulonglong("12"), to_ulonglong("x1"), to_ulonglong("")),
        (12, 0, 0)
    );
    assert_eq!(
        rpc_message(&rpc("FILE_REFERENCE_EXPIRED", 400)),
        "FILE_REFERENCE_EXPIRED"
    );
    assert_eq!(
        rpc_message(&rpc("LOCATION_INVALID", 400)),
        "LOCATION_INVALID"
    );
}
#[test]
fn action_photo_first() {
    let photo = Photo {
        image: Image {
            file: File {
                size: 11,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let doc = Document {
        is_video_file: true,
        file: File {
            size: 99,
            ..Default::default()
        },
        thumb: Image {
            width: 90,
            ..Default::default()
        },
        ..Default::default()
    };
    let m = Message {
        action: export::model_actions::ServiceAction {
            content: Some(Action::ChatEditPhoto(ActionChatEditPhoto { photo })),
        },
        media: Media {
            content: Some(MediaContent::Document(doc)),
            ..Default::default()
        },
        ..Default::default()
    };
    let w = message_file_work(&m);
    assert_eq!(w.len(), 2);
    assert_eq!(
        (w[0].file.size, w[0].media_type, w[0].controlling_size),
        (11, MediaSettings::PHOTO, 11)
    );
    assert_eq!(
        (w[1].media_type, w[1].controlling_size),
        (MediaSettings::VIDEO, 11)
    );
    assert_eq!(w[1].slot, FileSlot::Thumb);
}
#[test]
fn contact_and_voice_work() {
    let c = SharedContact {
        vcard: File {
            size: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let work = message_file_work(&Message {
        media: Media {
            content: Some(MediaContent::SharedContact(c)),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(
        (work.len(), work[0].file.size, work[0].media_type),
        (1, 3, MediaSettings::PHOTO)
    );
    let d = Document {
        is_voice_message: true,
        file: File {
            size: 4,
            ..Default::default()
        },
        ..Default::default()
    };
    let work = message_file_work(&Message {
        media: Media {
            content: Some(MediaContent::Document(d)),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(
        (work.len(), work[0].file.size, work[0].media_type),
        (1, 4, MediaSettings::VOICE_MESSAGE)
    );
    assert!(message_file_work(&Message::default()).is_empty());
}
#[test]
fn rich_work_order_and_identity_dedup() {
    let photo = Photo {
        id: 1,
        image: Image {
            file: File {
                size: 5,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let sticker = Document {
        id: 2,
        is_sticker: true,
        file: File {
            size: 6,
            ..Default::default()
        },
        thumb: Image {
            width: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let rich = RichMessage {
        blocks: vec![
            RichBlock {
                kind: RichBlockKind::Photo,
                photo_id: 1,
                ..Default::default()
            },
            RichBlock {
                kind: RichBlockKind::Photo,
                photo_id: 1,
                ..Default::default()
            },
            RichBlock {
                kind: RichBlockKind::File,
                document_id: 2,
                ..Default::default()
            },
        ],
        photos: [(1, photo)].into(),
        documents: [(2, sticker)].into(),
        ..Default::default()
    };
    let m = Message {
        media: Media {
            content: Some(MediaContent::Photo(Photo {
                image: Image {
                    file: File {
                        size: 1,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            })),
            ..Default::default()
        },
        rich_message: Some(rich),
        ..Default::default()
    };
    let work = message_file_work(&m);
    assert_eq!(
        work.iter()
            .map(|w| (w.slot.clone(), w.media_type, w.controlling_size, w.rich))
            .collect::<Vec<_>>(),
        [
            (FileSlot::Main, MediaSettings::PHOTO, 1, false),
            (FileSlot::RichPhoto(1), MediaSettings::PHOTO, 5, true),
            (FileSlot::RichDocument(2), MediaSettings::STICKER, 6, true),
            (FileSlot::RichThumb(2), MediaSettings::STICKER, 6, true)
        ]
    );
}
#[tokio::test]
async fn base_requests_refuse() {
    struct Abstract;
    impl ExportClient for Abstract {}
    let c = Abstract;
    assert!(matches!(
        c.request(RequestScope::default(), Value::Null).await,
        Err(FetchError::Local(export::ExportError::Unsupported))
    ));
    let s = RequestScope {
        range: Some(json!({})),
        ..Default::default()
    };
    assert!(c.request(s, Value::Null).await.is_err());
}

#[test]
fn collect_emoji_filters_known_zero_and_dates() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::default();
    let mut f = files(&c, d.path());
    f.settings.single_peer_from = 10;
    f.resolved_emoji.insert(8, Document::default());
    let inside = Message {
        date: 20,
        text: [5, 8, 0]
            .map(|i| TextPart {
                kind: TextType::CustomEmoji,
                additional: i.to_string(),
                ..Default::default()
            })
            .into(),
        reactions: vec![
            Reaction {
                r#type: ReactionType::CustomEmoji,
                document_id: "6".into(),
                ..Default::default()
            },
            Reaction {
                r#type: ReactionType::Emoji,
                document_id: "66".into(),
                ..Default::default()
            },
        ],
        rich_message: Some(RichMessage {
            blocks: vec![RichBlock {
                kind: RichBlockKind::Paragraph,
                text: rich_emoji(7),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let early = Message {
        date: 5,
        text: vec![TextPart {
            kind: TextType::CustomEmoji,
            additional: "9".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    f.collect_custom_emoji(&MessagesSlice {
        list: vec![inside, early],
        ..Default::default()
    });
    assert_eq!(f.unresolved_emoji, [5, 6, 7].into());
}
#[tokio::test]
async fn resolve_emoji_largest_first_and_failed_chunk() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(
        vec![Ok(json!([emoji(7, b"old")])), Err(rpc("X", 400))],
        vec![],
    );
    let mut f = files(&c, d.path());
    f.emoji_chunk = 1;
    f.unresolved_emoji = [5, 7].into();
    f.resolve_custom_emoji(&mut ParseMediaContext::default(), "", &DateContext::utc())
        .await
        .unwrap();
    let calls = c.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .map(|(_, r)| r["document_id"].clone())
            .collect::<Vec<_>>(),
        [json!([7]), json!([5])]
    );
    assert!(f.resolved_emoji[&7].is_sticker);
    assert_eq!(
        f.resolved_emoji[&5].file.skip_reason,
        SkipReason::Unavailable
    );
    assert_eq!(
        f.warnings,
        ["Export Error: Failed to get documents for emoji."]
    );
    assert!(f.unresolved_emoji.is_empty());
}
#[tokio::test]
async fn emoji_paths_placeholders_and_cache() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::default();
    let mut f = files(&c, d.path());
    let ready = Document {
        id: 3,
        is_sticker: true,
        file: File {
            content: b"webp".to_vec(),
            size: 4,
            suggested_path: "stickers/e.webp".into(),
            location: location(3, b"old", "", true),
            ..Default::default()
        },
        ..Default::default()
    };
    f.resolved_emoji = [
        (3, ready),
        (
            4,
            Document {
                file: File {
                    skip_reason: SkipReason::FileType,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
        (
            5,
            Document {
                file: File {
                    skip_reason: SkipReason::Unavailable,
                    ..Default::default()
                },
                ..Default::default()
            },
        ),
    ]
    .into();
    for (input, expected) in [
        ("abc", "abc"),
        ("0", "0"),
        ("9", "(unavailable)"),
        ("4", ""),
        ("5", "(unavailable)"),
        ("3", "stickers/e.webp"),
    ] {
        assert_eq!(
            f.custom_emoji_value("", &Message::default(), input, &DateContext::utc())
                .await
                .unwrap(),
            expected
        );
    }
    assert_eq!(fs::read(d.path().join("stickers/e.webp")).unwrap(), b"webp");
    f.resolved_emoji.insert(
        6,
        Document {
            file: File {
                content: b"x".to_vec(),
                location: location(3, b"old", "", true),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(
        f.custom_emoji_value("", &Message::default(), "6", &DateContext::utc())
            .await
            .unwrap(),
        "stickers/e.webp"
    );
}
#[tokio::test]
async fn emoji_replaced_everywhere() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::default();
    let mut f = files(&c, d.path());
    f.resolved_emoji.insert(
        3,
        Document {
            file: File {
                relative_path: "stickers/e.webp".into(),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let mut message = Message {
        text: vec![
            TextPart {
                kind: TextType::CustomEmoji,
                additional: "3".into(),
                ..Default::default()
            },
            TextPart::plain("t"),
        ],
        reactions: vec![Reaction {
            r#type: ReactionType::CustomEmoji,
            document_id: "9".into(),
            ..Default::default()
        }],
        rich_message: Some(RichMessage {
            blocks: vec![RichBlock {
                kind: RichBlockKind::Paragraph,
                text: rich_emoji(3),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    f.resolve_message_custom_emoji("", &mut message, &DateContext::utc())
        .await
        .unwrap();
    assert_eq!(message.text[0].additional, "stickers/e.webp");
    assert_eq!(message.text[1].additional, "");
    assert_eq!(message.reactions[0].document_id, "(unavailable)");
    assert_eq!(
        message.rich_message.unwrap().blocks[0]
            .text
            .custom_emoji_data,
        "stickers/e.webp"
    );
    let mut plain = Message {
        text: vec![TextPart {
            kind: TextType::CustomEmoji,
            additional: "3".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    f.resolve_message_custom_emoji("", &mut plain, &DateContext::utc())
        .await
        .unwrap();
    assert_eq!(plain.text[0].additional, "stickers/e.webp");
}

fn rich_emoji(id: i64) -> RichText {
    RichText {
        r#type: RichTextType::CustomEmoji,
        id,
        custom_emoji_data: id.to_string(),
        ..Default::default()
    }
}
