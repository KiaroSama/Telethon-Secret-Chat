#![allow(clippy::unwrap_used)]
#[path = "support_media/forward_review.rs"]
mod forward_review;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    Serializable, enums,
    files::FileKey,
    manager::{Event, MessageReceived, SendFileOptions},
    media::{self, FileSource, MediaKind, MediaReference, Metadata},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use support::*;
fn jpeg() -> Vec<u8> {
    let mut data = vec![255, 216, 255, 224];
    data.resize(64, 0);
    data
}
fn webp() -> Vec<u8> {
    let mut data = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    data.resize(66, 0);
    data
}
fn collected(manager: &Manager) -> Arc<Mutex<Vec<MessageReceived>>> {
    let records = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&records);
    manager
        .on("MessageReceived", "collect_media", move |event| {
            if let Event::MessageReceived(message) = event {
                sink.lock().unwrap().push(*message);
            }
            Ok(())
        })
        .unwrap();
    records
}
async fn sent(
    name: &str,
    kind: Option<MediaKind>,
    metadata: Metadata,
) -> (Manager, Manager, Arc<Mutex<World>>, i32, MessageReceived) {
    let (a, b, world, id) = established().await;
    let got = collected(&b);
    a.send_file(
        id,
        FileSource::bytes(vec![42; 5000], name).unwrap(),
        SendFileOptions {
            kind,
            metadata,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    let message = got.lock().unwrap()[0].clone();
    (a, b, world, id, message)
}
fn attributes(message: &MessageReceived) -> &[enums::DocumentAttribute] {
    match message.media.as_ref().unwrap() {
        enums::DecryptedMessageMedia::Document(m) => &m.attributes,
        enums::DecryptedMessageMedia::DocumentLegacy7afe8ae2(m) => &m.attributes,
        _ => panic!("Expected document-shaped media"),
    }
}
#[test]
fn package_names_all_eight_kinds() {
    assert_eq!(
        media::MEDIA_KINDS,
        [
            "photo",
            "video",
            "document",
            "audio",
            "animation",
            "sticker",
            "video_note",
            "voice_note"
        ]
    );
}
#[test]
fn two_kinds_are_captionless() {
    assert_eq!(media::CAPTIONLESS_KINDS, ["sticker", "video_note"]);
}
#[tokio::test]
async fn document_carries_only_filename() {
    let (_, _, _, _, m) = sent("report.txt", None, Metadata::default()).await;
    assert_eq!(attributes(&m).len(), 1);
    assert!(
        matches!(&attributes(&m)[0],enums::DocumentAttribute::Filename(n)if n.file_name=="report.txt")
    );
}
#[tokio::test]
async fn audio_has_audio_attribute() {
    let (_, _, _, _, m) = sent("song.mp3", None, Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Audio(a)if !a.voice))
    );
}
#[tokio::test]
async fn voice_note_sets_voice_flag() {
    let (_, _, _, _, m) = sent("note.ogg", Some(MediaKind::VoiceNote), Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Audio(a)if a.voice))
    );
}
#[tokio::test]
async fn video_has_video_attribute() {
    let (_, _, _, _, m) = sent("clip.mp4", None, Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Video(a)if !a.round_message))
    );
}
#[tokio::test]
async fn video_note_sets_round_flag() {
    let (_, _, _, _, m) = sent("round.mp4", Some(MediaKind::VideoNote), Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Video(a)if a.round_message))
    );
}
#[tokio::test]
async fn photo_has_image_size_attribute() {
    let (_, _, _, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a, enums::DocumentAttribute::ImageSize(_)))
    );
}
#[tokio::test]
async fn animation_is_marked_animated() {
    let (_, _, _, _, m) = sent("loop.gif", None, Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a, enums::DocumentAttribute::Animated))
    );
}
#[tokio::test]
async fn sticker_has_sticker_attribute() {
    let (_, _, _, _, m) = sent("face.webp", Some(MediaKind::Sticker), Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a, enums::DocumentAttribute::Sticker(_)))
    );
}
#[tokio::test]
async fn explicit_document_overrides_inference() {
    let (_, _, _, _, m) = sent("song.mp3", Some(MediaKind::Document), Metadata::default()).await;
    assert_eq!(attributes(&m).len(), 1);
    assert!(matches!(
        attributes(&m)[0],
        enums::DocumentAttribute::Filename(_)
    ));
}
#[tokio::test]
async fn every_kind_keeps_filename() {
    let (a, b, world, id) = established().await;
    let got = collected(&b);
    for (name, kind) in [
        ("a.mp3", MediaKind::Audio),
        ("b.ogg", MediaKind::VoiceNote),
        ("c.mp4", MediaKind::Video),
        ("d.mp4", MediaKind::VideoNote),
        ("e.jpg", MediaKind::Photo),
        ("f.gif", MediaKind::Animation),
        ("g.webp", MediaKind::Sticker),
        ("h.bin", MediaKind::Document),
    ] {
        a.send_file(
            id,
            FileSource::bytes(vec![1; 32], name).unwrap(),
            SendFileOptions {
                kind: Some(kind),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        pump(&a, &b, &world).await;
        assert!(
            attributes(got.lock().unwrap().last().unwrap())
                .iter()
                .any(|a| matches!(a,enums::DocumentAttribute::Filename(n)if n.file_name==name))
        );
    }
}
#[tokio::test]
async fn impossible_kind_refused_before_upload() {
    let (a, _, world, id) = established().await;
    let error = a
        .send_file(
            id,
            FileSource::bytes(vec![1; 32], "song.mp3").unwrap(),
            SendFileOptions {
                kind: Some(MediaKind::Video),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("song.mp3"));
    assert!(error.to_string().contains("video"));
    assert_eq!(world.lock().unwrap().uploads, 0);
}
#[tokio::test]
async fn photo_as_voice_refused_before_upload() {
    let (a, _, world, id) = established().await;
    assert!(
        a.send_file(
            id,
            FileSource::bytes(vec![1; 32], "picture.jpg").unwrap(),
            SendFileOptions {
                kind: Some(MediaKind::VoiceNote),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(world.lock().unwrap().uploads, 0);
}
#[test]
fn unknown_kind_is_refused() {
    assert!(
        MediaKind::parse("hologram")
            .unwrap_err()
            .to_string()
            .contains("hologram")
    );
}
#[tokio::test]
async fn captionless_kinds_refuse_caption() {
    let (a, _, world, id) = established().await;
    for (name, kind) in [
        ("face.webp", MediaKind::Sticker),
        ("round.mp4", MediaKind::VideoNote),
    ] {
        let error = a
            .send_file(
                id,
                FileSource::bytes(vec![1; 32], name).unwrap(),
                SendFileOptions {
                    kind: Some(kind),
                    caption: "look".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(kind.as_str()));
    }
    assert_eq!(world.lock().unwrap().uploads, 0);
}
#[tokio::test]
async fn audio_keeps_caption() {
    let (a, b, world, id) = established().await;
    let got = collected(&b);
    a.send_file(
        id,
        FileSource::bytes(vec![1; 32], "song.mp3").unwrap(),
        SendFileOptions {
            caption: "demo".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(got.lock().unwrap()[0].text, "demo");
    assert!(
        matches!(got.lock().unwrap()[0].media.as_ref(),Some(enums::DecryptedMessageMedia::Document(m))if m.caption=="demo")
    );
}
#[tokio::test]
async fn video_metadata_and_preview_survive() {
    let (_, _, _, _, m) = sent(
        "clip.mp4",
        None,
        Metadata {
            duration: Some(3),
            width: Some(320),
            height: Some(240),
            thumbnail: Some(jpeg()),
            thumbnail_size: Some((90, 68)),
            ..Default::default()
        },
    )
    .await;
    assert!(attributes(&m).iter().any(
        |a| matches!(a,enums::DocumentAttribute::Video(a)if(a.duration,a.w,a.h)==(3,320,240))
    ));
    assert!(
        matches!(m.media.as_ref(),Some(enums::DecryptedMessageMedia::Document(d))if(d.thumb.clone(),d.thumb_w,d.thumb_h)==(jpeg(),90,68))
    );
}
#[tokio::test]
async fn voice_waveform_survives() {
    let (_, _, _, _, m) = sent(
        "note.ogg",
        Some(MediaKind::VoiceNote),
        Metadata {
            duration: Some(7),
            waveform: Some(vec![31; 63]),
            ..Default::default()
        },
    )
    .await;
    assert!(attributes(&m).iter().any(|a|matches!(a,enums::DocumentAttribute::Audio(a)if a.voice&&a.duration==7&&a.waveform==Some(vec![31;63]))));
}
#[tokio::test]
async fn song_metadata_survives() {
    let (_, _, _, _, m) = sent(
        "song.mp3",
        None,
        Metadata {
            duration: Some(200),
            title: Some("T".into()),
            performer: Some("P".into()),
            ..Default::default()
        },
    )
    .await;
    assert!(attributes(&m).iter().any(|a|matches!(a,enums::DocumentAttribute::Audio(a)if a.duration==200&&a.title.as_deref()==Some("T")&&a.performer.as_deref()==Some("P"))));
}
#[tokio::test]
async fn sticker_metadata_survives() {
    let (_, _, _, _, m) = sent(
        "s.webp",
        Some(MediaKind::Sticker),
        Metadata {
            width: Some(512),
            height: Some(512),
            sticker_alt: Some("🙂".into()),
            thumbnail: Some(webp()),
            thumbnail_size: Some((128, 128)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Sticker(a)if a.alt=="🙂"))
    );
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::ImageSize(a)if(a.w,a.h)==(512,512)))
    );
}
#[tokio::test]
async fn photo_metadata_survives() {
    let (_, _, _, _, m) = sent(
        "p.jpg",
        None,
        Metadata {
            width: Some(800),
            height: Some(600),
            ..Default::default()
        },
    )
    .await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::ImageSize(a)if(a.w,a.h)==(800,600)))
    );
}
#[tokio::test]
async fn omitted_metadata_changes_nothing() {
    let (_, _, _, _, m) = sent("clip.mp4", None, Metadata::default()).await;
    assert!(
        attributes(&m)
            .iter()
            .any(|a| matches!(a,enums::DocumentAttribute::Video(a)if(a.duration,a.w,a.h)==(0,0,0)))
    );
    assert!(
        matches!(m.media.as_ref(),Some(enums::DecryptedMessageMedia::Document(d))if d.thumb.is_empty()&&d.thumb_w==0&&d.thumb_h==0)
    );
}
fn bad_metadata(index: usize) -> (MediaKind, Value) {
    let values = vec![
        (MediaKind::Photo, json!({"waveform":[1]})),
        (MediaKind::Photo, json!({"duration":3})),
        (MediaKind::Document, json!({"width":10})),
        (MediaKind::Audio, json!({"sticker_alt":"x"})),
        (MediaKind::Video, json!({"title":"T"})),
        (MediaKind::Video, json!({"duration":-1})),
        (MediaKind::Video, json!({"width":2147483648u64})),
        (MediaKind::Video, json!({"duration":true})),
        (MediaKind::Video, json!({"thumbnail":jpeg()})),
        (MediaKind::Video, json!({"thumbnail_size":[90,68]})),
        (
            MediaKind::Video,
            json!({"thumbnail":jpeg(),"thumbnail_size":[321,10]}),
        ),
        (
            MediaKind::Video,
            json!({"thumbnail":jpeg(),"thumbnail_size":[0,10]}),
        ),
        (
            MediaKind::Video,
            json!({"thumbnail":vec![0;200*1024+3],"thumbnail_size":[9,9]}),
        ),
        (
            MediaKind::Video,
            json!({"thumbnail":b"not an image".to_vec(),"thumbnail_size":[9,9]}),
        ),
        (
            MediaKind::Video,
            json!({"thumbnail":webp(),"thumbnail_size":[9,9]}),
        ),
        (MediaKind::VoiceNote, json!({"waveform":vec![0;64]})),
        (MediaKind::Audio, json!({"title":5})),
    ];
    values.into_iter().nth(index).unwrap()
}
async fn refused_metadata(index: usize) {
    let (kind, value) = bad_metadata(index);
    assert!(Metadata::from_json(value.clone(), kind).is_err());
    if let Ok(metadata) = serde_json::from_value::<Metadata>(value) {
        let (a, _, world, id) = established().await;
        let name = match kind {
            MediaKind::Photo => "image.jpg",
            MediaKind::Audio => "song.mp3",
            MediaKind::VoiceNote => "voice.ogg",
            MediaKind::Document => "data.bin",
            _ => "video.mp4",
        };
        assert!(
            a.send_file(
                id,
                FileSource::bytes(vec![1; 32], name).unwrap(),
                SendFileOptions {
                    kind: Some(kind),
                    metadata,
                    ..Default::default()
                }
            )
            .await
            .is_err()
        );
        assert_eq!(world.lock().unwrap().uploads, 0);
    }
}
macro_rules! bad_meta{($($name:ident=$n:expr),*)=>{$(#[tokio::test]async fn $name(){refused_metadata($n).await;})*};}
bad_meta!(
    bad_metadata_00 = 0,
    bad_metadata_01 = 1,
    bad_metadata_02 = 2,
    bad_metadata_03 = 3,
    bad_metadata_04 = 4,
    bad_metadata_05 = 5,
    bad_metadata_06 = 6,
    bad_metadata_07 = 7,
    bad_metadata_08 = 8,
    bad_metadata_09 = 9,
    bad_metadata_10 = 10,
    bad_metadata_11 = 11,
    bad_metadata_12 = 12,
    bad_metadata_13 = 13,
    bad_metadata_14 = 14,
    bad_metadata_15 = 15,
    bad_metadata_16 = 16
);
#[tokio::test]
async fn only_file_message_carries_reference() {
    let (a, b, world, id, m) = sent("photo.jpg", None, Metadata::default()).await;
    assert!(m.media_reference.is_some());
    let got = collected(&b);
    a.send_message(id, "text", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert!(got.lock().unwrap()[0].media_reference.is_none());
}
#[tokio::test]
async fn reference_roundtrips_plain_data() {
    let (_, _, _, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    let reference = m.media_reference.unwrap();
    let restored = MediaReference::from_json(
        serde_json::from_str(&serde_json::to_string(&reference.to_json().unwrap()).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(restored, reference);
    assert_eq!(restored.size().unwrap(), 5000);
    assert_eq!(restored.mime_type().unwrap().as_deref(), Some("image/jpeg"));
}
#[tokio::test]
async fn saving_reference_writes_original_bytes() {
    let (_, b, _, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ref.jpg");
    b.save_file(m.media_reference.as_ref().unwrap(), &path)
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), vec![42; 5000]);
}
#[tokio::test]
async fn wrong_reference_fingerprint_writes_nothing() {
    let (_, b, world, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    let mut reference = m.media_reference.unwrap();
    let mut data = reference.to_json().unwrap();
    data["key_fingerprint"] = json!(data["key_fingerprint"].as_i64().unwrap() ^ 1);
    assert!(MediaReference::from_json(data).is_err());
    reference.key_fingerprint ^= 1;
    let folder = tempfile::tempdir().unwrap();
    let destination = folder.path().join("missing").join("file.jpg");
    assert!(b.save_file(&reference, &destination).await.is_err());
    assert!(!destination.parent().unwrap().exists());
    assert_eq!(world.lock().unwrap().downloads, 0);
}
async fn damaged_reference(index: usize) {
    let (_, _, _, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    let mut data = m.media_reference.unwrap().to_json().unwrap();
    let changes = [
        ("version", json!(2)),
        ("chat_id", json!("1")),
        ("file_id", Value::Null),
        ("media", json!("zz")),
        ("media", json!("00")),
    ];
    data[changes[index].0] = changes[index].1.clone();
    let error = MediaReference::from_json(data).unwrap_err();
    assert!(!error.to_string().contains("2a2a2a"));
}
macro_rules! bad_reference{($($name:ident=$n:expr),*)=>{$(#[tokio::test]async fn $name(){damaged_reference($n).await;})*};}
bad_reference!(
    damaged_reference_0 = 0,
    damaged_reference_1 = 1,
    damaged_reference_2 = 2,
    damaged_reference_3 = 3,
    damaged_reference_4 = 4
);
#[tokio::test]
async fn reference_debug_never_reveals_key() {
    let (_, b, _, _, m) = sent("photo.jpg", None, Metadata::default()).await;
    let reference = m.media_reference.as_ref().unwrap();
    let parts = reference.validate().unwrap();
    let mut wrong = reference.clone();
    wrong.key_fingerprint ^= 1;
    let folder = tempfile::tempdir().unwrap();
    let error = b
        .save_file(&wrong, &folder.path().join("never"))
        .await
        .unwrap_err();
    let debug = format!("{reference:?} {m:?} {error} {error:?}");
    for secret in [parts.key.key(), parts.key.iv()] {
        let hex = hex::encode(secret);
        assert!(!debug.contains(&hex));
        assert!(!debug.contains(&hex[..16]));
    }
}
#[tokio::test]
async fn forward_reference_moves_no_bytes() {
    huge_file_requires_layer143();
    forward(false).await;
}
#[tokio::test]
async fn forward_message_moves_no_bytes() {
    forward_review::forward_observes_ttl_after_retry().await;
    old_peer_uses_old_document_shape();
    forward(true).await;
}
async fn forward(as_message: bool) {
    let (a, b, world, old_id, m) = sent(
        "clip.mp4",
        None,
        Metadata {
            duration: Some(3),
            width: Some(320),
            height: Some(240),
            thumbnail: Some(jpeg()),
            thumbnail_size: Some((90, 68)),
            ..Default::default()
        },
    )
    .await;
    let id = b.create(peer(1000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    a.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_ne!(old_id, id);
    let got = collected(&a);
    let (before_upload, before_download) = {
        let w = world.lock().unwrap();
        (w.uploads, w.downloads)
    };
    if as_message {
        b.forward_message_file(id, &m, "fwd", Some(42))
            .await
            .unwrap();
    } else {
        b.forward_file(id, m.media_reference.as_ref().unwrap(), "fwd", Some(42))
            .await
            .unwrap();
    }
    pump(&a, &b, &world).await;
    assert_eq!(
        {
            let w = world.lock().unwrap();
            (w.uploads, w.downloads)
        },
        (before_upload, before_download)
    );
    let arrived = got.lock().unwrap()[0].clone();
    assert_eq!(arrived.chat_id, id);
    assert_eq!(arrived.text, "fwd");
    assert_eq!(arrived.reply_to, Some(42));
    let original = media::file_parts(m.media.as_ref().unwrap()).unwrap();
    let forwarded = media::file_parts(arrived.media.as_ref().unwrap()).unwrap();
    assert_eq!(original.key.key(), forwarded.key.key());
    assert_eq!(original.key.iv(), forwarded.key.iv());
    assert_eq!(original.size, forwarded.size);
    assert_eq!(original.mime_type, forwarded.mime_type);
    assert_eq!(
        attributes(&m)
            .iter()
            .map(Serializable::to_bytes)
            .collect::<Vec<_>>(),
        attributes(&arrived)
            .iter()
            .map(Serializable::to_bytes)
            .collect::<Vec<_>>()
    );
    let original_ref = m.media_reference.as_ref().unwrap();
    let forwarded_ref = arrived.media_reference.as_ref().unwrap();
    assert_eq!(
        (forwarded_ref.file_id, forwarded_ref.access_hash),
        (original_ref.file_id, original_ref.access_hash)
    );
    if let (
        Some(enums::DecryptedMessageMedia::Document(a)),
        Some(enums::DecryptedMessageMedia::Document(b)),
    ) = (&m.media, &arrived.media)
    {
        assert_eq!(
            (&a.thumb, a.thumb_w, a.thumb_h),
            (&b.thumb, b.thumb_w, b.thumb_h)
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("forwarded.mp4");
    a.save_message_file(&arrived, &destination).await.unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), vec![42; 5000]);
}
#[tokio::test]
async fn forward_without_file_is_refused_before_sending() {
    let (a, b, world, id) = established().await;
    let got = collected(&b);
    a.send_message(id, "plain", Some(vec![]), None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let before = world.lock().unwrap().sent.len();
    let message = got.lock().unwrap()[0].clone();
    assert!(
        b.forward_message_file(id, &message, "", None)
            .await
            .is_err()
    );
    assert_eq!(world.lock().unwrap().sent.len(), before);
}
fn huge_file_requires_layer143() {
    let key = FileKey::generate().unwrap();
    let media = media::document_media(media::DocumentMediaInput {
        kind: MediaKind::Document,
        name: "large.bin",
        mime: "application/octet-stream",
        size: (1u64 << 31) + 1,
        layer: 144,
        key: &key,
        caption: "",
        metadata: &Metadata::default(),
    })
    .unwrap();
    let reference =
        MediaReference::from_parts(1, 77, 88, 1, key.fingerprint(), media.to_bytes()).unwrap();
    assert!(media::forwarded_media(&reference, 101, "").is_err());
    assert!(media::forwarded_media(&reference, 144, "").is_ok());
}
fn old_peer_uses_old_document_shape() {
    let key = FileKey::generate().unwrap();
    let media = media::document_media(media::DocumentMediaInput {
        kind: MediaKind::Document,
        name: "file.bin",
        mime: "application/octet-stream",
        size: 10,
        layer: 144,
        key: &key,
        caption: "",
        metadata: &Metadata::default(),
    })
    .unwrap();
    let reference =
        MediaReference::from_parts(1, 77, 88, 1, key.fingerprint(), media.to_bytes()).unwrap();
    assert!(matches!(
        media::forwarded_media(&reference, 101, "").unwrap(),
        enums::DecryptedMessageMedia::DocumentLegacy7afe8ae2(_)
    ));
}
