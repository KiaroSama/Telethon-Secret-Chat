#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use grammers_tl_types as tl;
use kiaro_secret_chat::{
    Serializable, enums,
    files::{FileKey, encrypt_file},
    manager::{Event, MessageReceived, Options, SecretChatManager, SendFileOptions},
    media::{self, FileSource, MediaKind, MediaReference, Metadata},
    storage::MemoryStorage,
    types,
};
use std::sync::{Arc, Mutex};
use support::*;
fn reference() -> MediaReference {
    let key = FileKey::generate().unwrap();
    let media = media::document_media(media::DocumentMediaInput {
        kind: MediaKind::Document,
        name: "file.bin",
        mime: "application/octet-stream",
        size: 16,
        layer: 144,
        key: &key,
        caption: "",
        metadata: &Metadata::default(),
    })
    .unwrap();
    MediaReference::from_parts(7, 77, 99, 1, key.fingerprint(), media.to_bytes()).unwrap()
}
fn message(reference: &MediaReference) -> MessageReceived {
    MessageReceived {
        chat_id: reference.chat_id,
        random_id: 1,
        seq_no: 1,
        text: String::new(),
        entities: None,
        ttl: 0,
        media: Some(reference.decoded().unwrap()),
        file: Some(tl::types::EncryptedFile {
            id: reference.file_id,
            access_hash: reference.access_hash,
            size: 16,
            dc_id: 1,
            key_fingerprint: reference.key_fingerprint,
        }),
        reply_to: None,
        media_reference: Some(reference.clone()),
    }
}
async fn damaged_forward(as_message: bool) {
    let (a, _b, world, id) = established().await;
    let original = reference();
    let mut damaged = original.clone();
    damaged.key_fingerprint ^= 1;
    let before = world.lock().unwrap().sent.len();
    if as_message {
        let mut forged = message(&original);
        forged.file.as_mut().unwrap().key_fingerprint ^= 1;
        assert!(a.forward_message_file(id, &forged, "", None).await.is_err());
    } else {
        assert!(a.forward_file(id, &damaged, "", None).await.is_err());
    }
    assert_eq!(world.lock().unwrap().sent.len(), before);
    a.send_message(id, "no sequence reserved", None, None)
        .await
        .unwrap();
    let after = world.lock().unwrap().sent.last().unwrap().outgoing.seq_no;
    assert_eq!(after, 3);
}
#[tokio::test]
async fn forwarding_bad_reference_fingerprint_reserves_nothing() {
    damaged_forward(false).await;
}
#[tokio::test]
async fn forwarding_bad_message_fingerprint_reserves_nothing() {
    damaged_forward(true).await;
}
async fn captionless(kind: MediaKind) {
    let (a, _b, world, id) = established().await;
    let source = reference();
    let mut media = source.decoded().unwrap();
    if let enums::DecryptedMessageMedia::Document(d) = &mut media {
        d.attributes = media::attributes_for(kind, "item.bin", &Metadata::default()).unwrap();
    }
    let source = MediaReference::from_parts(
        source.chat_id,
        source.file_id,
        source.access_hash,
        source.dc_id,
        source.key_fingerprint,
        media.to_bytes(),
    )
    .unwrap();
    let before = world.lock().unwrap().sent.len();
    let error = a
        .forward_file(id, &source, "must not disappear", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("caption"));
    assert_eq!(world.lock().unwrap().sent.len(), before);
}
#[tokio::test]
async fn forwarding_sticker_refuses_caption() {
    captionless(MediaKind::Sticker).await;
}
#[tokio::test]
async fn forwarding_round_video_refuses_caption() {
    captionless(MediaKind::VideoNote).await;
}
fn unusable(change: usize) {
    let source = reference();
    let mut data = source.to_json().unwrap();
    match change {
        0 => {
            let mut media = source.decoded().unwrap();
            if let enums::DecryptedMessageMedia::Document(d) = &mut media {
                d.size = -1;
            }
            data["media"] = hex::encode(media.to_bytes()).into();
        }
        1 => {
            data["media"] = format!("{}00000000", data["media"].as_str().unwrap()).into();
        }
        2 => {
            data["version"] = true.into();
        }
        _ => {
            data["key_fingerprint"] = (source.key_fingerprint ^ 1).into();
        }
    }
    assert!(MediaReference::from_json(data).is_err());
}
#[test]
fn reference_refuses_negative_size() {
    unusable(0);
}
#[test]
fn reference_refuses_trailing_data() {
    unusable(1);
}
#[test]
fn reference_refuses_boolean_version() {
    unusable(2);
}
#[test]
fn reference_refuses_bad_persisted_fingerprint() {
    unusable(3);
}
#[test]
fn reference_decodes_one_complete_file_media_object() {
    let source = reference();
    let decoded = source.decoded().unwrap();
    assert_eq!(decoded.to_bytes(), source.media_bytes());
    assert_eq!(
        MediaReference::from_json(source.to_json().unwrap())
            .unwrap()
            .decoded()
            .unwrap()
            .to_bytes(),
        decoded.to_bytes()
    );
}
#[tokio::test]
async fn forwarded_photo_remains_photo_and_decrypts_correctly() {
    let (a, b, world, id) = established().await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    b.on("MessageReceived", "photo", move |e| {
        if let Event::MessageReceived(m) = e {
            sink.lock().unwrap().push(m);
        }
        Ok(())
    })
    .unwrap();
    let key = FileKey::generate().unwrap();
    let photo = enums::DecryptedMessageMedia::from(types::DecryptedMessageMediaPhoto {
        thumb: vec![],
        thumb_w: 0,
        thumb_h: 0,
        w: 10,
        h: 10,
        size: 16,
        key: key.key().to_vec(),
        iv: key.iv().to_vec(),
        caption: String::new(),
    });
    let mut cipher = Vec::new();
    encrypt_file(&mut [b'p'; 16].as_slice(), &mut cipher, 16, &key).unwrap();
    {
        let mut w = world.lock().unwrap();
        w.files.insert(77, cipher);
        w.fingerprints.insert(77, key.fingerprint());
    }
    let reference =
        MediaReference::from_parts(1, 77, 99, 1, key.fingerprint(), photo.to_bytes()).unwrap();
    a.forward_file(id, &reference, "", None).await.unwrap();
    pump(&a, &b, &world).await;
    let arrived = got.lock().unwrap()[0].clone();
    assert!(matches!(
        arrived.media,
        Some(enums::DecryptedMessageMedia::Photo(_))
    ));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("photo.jpg");
    b.save_message_file(&arrived, &path).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), [b'p'; 16]);
}
async fn two_chats_with_source() -> (
    Manager,
    SecretChatManager<TestStorage, Fake>,
    TestStorage,
    Fake,
    Arc<Mutex<World>>,
    i32,
    MessageReceived,
) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let transport = bb.clone();
    let store = TestStorage::default();
    let a = SecretChatManager::new(
        aa,
        MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(bb, store.clone(), "22".repeat(32), Options::default()).unwrap();
    let first = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(first).await.unwrap();
    pump(&a, &b, &world).await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    b.on("MessageReceived", "source", move |event| {
        if let Event::MessageReceived(m) = event {
            sink.lock().unwrap().push(m);
        }
        Ok(())
    })
    .unwrap();
    a.send_file(
        first,
        FileSource::bytes(vec![7; 16], "file.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    let source = *got.lock().unwrap()[0].clone();
    let second = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(second).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, store, transport, world, second, source)
}
#[tokio::test]
async fn forward_retains_exact_server_handle_after_confirmation() {
    let (_a, b, store, _transport, _world, id, source) = two_chats_with_source().await;
    let random_id = b.forward_message_file(id, &source, "", None).await.unwrap();
    let row = store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
        .find(|r| r.random_id == random_id)
        .unwrap()
        .clone();
    assert!(!row.pending);
    let expected = tl::enums::InputEncryptedFile::from(tl::types::InputEncryptedFile {
        id: source.file.as_ref().unwrap().id,
        access_hash: source.file.as_ref().unwrap().access_hash,
    });
    assert_eq!(row.file.unwrap(), expected.to_bytes());
}
#[tokio::test]
async fn sent_file_retains_confirmed_server_handle_for_resend() {
    let (_a, b, store, _transport, world, id, _source) = two_chats_with_source().await;
    let random_id = b
        .send_file(
            id,
            FileSource::bytes(vec![2; 32], "notes.txt").unwrap(),
            SendFileOptions::default(),
        )
        .await
        .unwrap();
    let row = store.state.lock().unwrap().as_ref().unwrap().chats[&id]
        .outbox
        .values()
        .find(|r| r.random_id == random_id)
        .unwrap()
        .clone();
    let file_id = world.lock().unwrap().next_file - 1;
    let expected = tl::enums::InputEncryptedFile::from(tl::types::InputEncryptedFile {
        id: file_id,
        access_hash: 99,
    });
    assert_eq!(row.file.unwrap(), expected.to_bytes());
    assert!(!row.pending);
}
fn lower_layer(store: &TestStorage, id: i32) {
    let mut protocol =
        kiaro_secret_chat::protocol::Protocol::open(store.clone(), "22".repeat(32)).unwrap();
    protocol.start().unwrap();
    protocol
        .mutate_chat(id, |chat| {
            chat.capability_layer = 101;
            Ok(())
        })
        .unwrap();
    protocol.stop().unwrap();
}
#[tokio::test]
async fn huge_forward_to_old_peer_is_refused_before_reservation() {
    let (_a, b, store, transport, world, id, source) = two_chats_with_source().await;
    drop(b);
    lower_layer(&store, id);
    let b = SecretChatManager::new(
        transport,
        store.clone(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    let original = source.media_reference.unwrap();
    let mut media = original.decoded().unwrap();
    if let enums::DecryptedMessageMedia::Document(d) = &mut media {
        d.size = (1i64 << 31) + 1;
    }
    let reference = MediaReference::from_parts(
        original.chat_id,
        original.file_id,
        original.access_hash,
        original.dc_id,
        original.key_fingerprint,
        media.to_bytes(),
    )
    .unwrap();
    let before = store.state.lock().unwrap().as_ref().unwrap().chats[&id].out_seq_no;
    let sent = world.lock().unwrap().sent.len();
    assert!(b.forward_file(id, &reference, "", None).await.is_err());
    assert_eq!(world.lock().unwrap().sent.len(), sent);
    assert_eq!(
        store.state.lock().unwrap().as_ref().unwrap().chats[&id].out_seq_no,
        before
    );
}
#[tokio::test]
async fn forwarding_to_old_peer_sends_legacy_document_and_original_bytes() {
    let (a, b, store, transport, world, id, source) = two_chats_with_source().await;
    drop(b);
    lower_layer(&store, id);
    let b = SecretChatManager::new(transport, store, "22".repeat(32), Options::default()).unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    a.on("MessageReceived", "older", move |e| {
        if let Event::MessageReceived(m) = e {
            sink.lock().unwrap().push(m);
        }
        Ok(())
    })
    .unwrap();
    b.forward_message_file(id, &source, "", None).await.unwrap();
    pump(&a, &b, &world).await;
    let arrived = got.lock().unwrap()[0].clone();
    assert!(matches!(
        arrived.media,
        Some(enums::DecryptedMessageMedia::DocumentLegacy7afe8ae2(_))
    ));
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("old.bin");
    a.save_message_file(&arrived, &target).await.unwrap();
    assert_eq!(std::fs::read(target).unwrap(), vec![7; 16]);
}
#[tokio::test]
async fn file_reply_arrives_with_original_reply_id() {
    let (a, b, world, id) = established().await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    b.on("MessageReceived", "reply", move |e| {
        if let Event::MessageReceived(m) = e {
            sink.lock().unwrap().push(m);
        }
        Ok(())
    })
    .unwrap();
    a.send_file(
        id,
        FileSource::bytes(vec![7; 16], "reply.bin").unwrap(),
        SendFileOptions {
            reply_to: Some(123),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(got.lock().unwrap()[0].reply_to, Some(123));
}
