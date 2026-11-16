#![allow(clippy::unwrap_used)]
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    Deserializable,
    enums::{DecryptedMessage, DecryptedMessageMedia},
    manager::{Event, Options, SecretChatManager, SendFileOptions},
    media::{FileSource, MediaKind},
    storage::{MemoryStorage, Storage},
};
use seeded::*;
use std::sync::{Arc, Mutex};
use support::*;
async fn negotiated(layer: i32) {
    let mut record = chat();
    record.capability_layer = layer;
    record.wrapper_layer = layer;
    let (a, _store, w, id) = ready_chat(record, Options::default()).await;
    a.send_file(
        id,
        FileSource::bytes(b"sample".to_vec(), "one.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    let wire = w
        .lock()
        .unwrap()
        .sent
        .last()
        .unwrap()
        .outgoing
        .frame
        .clone();
    let plain = kiaro_secret_chat::crypto::decrypt_frame(
        &kiaro_secret_chat::crypto::SharedKey::from_slice(&[7; 256]).unwrap(),
        &wire,
        kiaro_secret_chat::crypto::Direction::Initiator,
    )
    .unwrap();
    let message = kiaro_secret_chat::framing::unwrap(&plain).unwrap().message;
    let media = match message {
        DecryptedMessage::Message(message) => message.media.unwrap(),
        DecryptedMessage::Legacy36b091de(message) => message.media.unwrap(),
        _ => panic!("expected content"),
    };
    if layer < 143 {
        assert!(matches!(
            media,
            DecryptedMessageMedia::DocumentLegacy7afe8ae2(_)
        ));
    } else {
        assert!(matches!(media, DecryptedMessageMedia::Document(_)));
    }
    a.stop().await.unwrap();
}
#[tokio::test]
async fn layer73_upload_uses_legacy_document_constructor() {
    negotiated(73).await;
}
#[tokio::test]
async fn layer143_upload_uses_current_document_constructor() {
    negotiated(143).await;
}
#[tokio::test]
async fn big_uploader_result_preserves_big_encrypted_handle() {
    let (a, _store, w, id) = ready(Options::default()).await;
    w.lock().unwrap().big_upload = true;
    a.send_file(
        id,
        FileSource::bytes(
            b"small stand-in for uploader big result".to_vec(),
            "small.bin",
        )
        .unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    let input = w
        .lock()
        .unwrap()
        .sent
        .last()
        .unwrap()
        .outgoing
        .file
        .clone()
        .unwrap();
    let file = grammers_tl_types::enums::InputEncryptedFile::from_bytes(&input).unwrap();
    assert!(
        matches!(file,grammers_tl_types::enums::InputEncryptedFile::BigUploaded(file) if file.parts==22)
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn native_reference_passes_file_dc_and_rejects_bad_fingerprint_before_download() {
    let (a, b, w, id) = established().await;
    w.lock().unwrap().file_dc = Some(4);
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    a.on("MessageReceived", "file", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message);
        }
        Ok(())
    })
    .unwrap();
    b.send_file(
        id,
        FileSource::bytes(b"sample".to_vec(), "one.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &w).await;
    let mut message = *events.lock().unwrap()[0].clone();
    let reference = message.media_reference.as_ref().unwrap();
    assert_eq!(
        grammers_client::media::Downloadable::dc_id(reference),
        Some(4)
    );
    let dir = tempfile::tempdir().unwrap();
    let saved = a
        .save_message_file(&message, &dir.path().join("ok"))
        .await
        .unwrap();
    assert_eq!(std::fs::read(saved).unwrap(), b"sample");
    assert_eq!(w.lock().unwrap().download_dcs, vec![4]);
    message.file.as_mut().unwrap().key_fingerprint ^= 1;
    assert!(
        a.save_message_file(&message, &dir.path().join("bad"))
            .await
            .is_err()
    );
    assert!(!dir.path().join("bad").exists());
    assert_eq!(w.lock().unwrap().download_dcs, vec![4]);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
fn bad_voice_mime(mime: &str) {
    assert!(
        kiaro_secret_chat::media::resolve_kind(Some(MediaKind::VoiceNote), mime, "", &[]).is_err()
    );
}
#[test]
fn voice_refuses_ogg_mime_prefix_extension() {
    bad_voice_mime("audio/ogg-invalid");
}
#[test]
fn voice_refuses_opus_mime_prefix_extension() {
    bad_voice_mime("audio/opus-invalid");
}
#[test]
fn voice_refuses_x_opus_mime_prefix_extension() {
    bad_voice_mime("audio/x-opus-invalid");
}
#[tokio::test]
async fn initial_visual_hash_is_distinct_from_wire_fingerprint_and_survives_rekey() {
    use sha1::Digest;
    let (aa, bb) = Fake::pair();
    let w = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let a = SecretChatManager::new(aa, store.clone(), "11".repeat(32), Options::default()).unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &w).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &w).await;
    let original = store.load().unwrap().unwrap().chats[&id]
        .key
        .clone()
        .unwrap();
    let expected = [
        &sha1::Sha1::digest(original.expose())[..16],
        &sha2::Sha256::digest(original.expose())[..20],
    ]
    .concat();
    assert_eq!(a.status(id).unwrap().key_hash, Some(expected.clone()));
    assert_ne!(expected.len(), 8);
    a.rekey(id).await.unwrap();
    pump(&a, &b, &w).await;
    assert_eq!(a.status(id).unwrap().key_hash, Some(expected.clone()));
    assert_eq!(
        store.load().unwrap().unwrap().chats[&id].initial_key_hash,
        Some(expected)
    );
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[test]
fn service_event_debug_omits_action_representation() {
    let sentinel = "SYNTHETIC-DO-NOT-PRINT";
    let event = Event::ServiceActionReceived {
        chat_id: 1,
        action_name: "action".into(),
        action: kiaro_secret_chat::types::DecryptedMessageActionRequestKey {
            exchange_id: 7,
            g_a: sentinel.as_bytes().to_vec(),
        }
        .into(),
        applied: false,
    };
    assert!(!format!("{event:?}").contains(sentinel));
}
