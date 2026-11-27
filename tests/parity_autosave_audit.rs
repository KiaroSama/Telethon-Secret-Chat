#![forbid(unsafe_code)]

use std::{fs, path::Path};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use kiaro_secret_chat::{
    autosave::{AutoSaveStore, SavedMediaRecord, SavedSecretMessage},
    errors::SecretChatError,
    media::SecretMediaKind,
};
use serde_json::{Value, json};
use tempfile::tempdir;

const KEY: [u8; 32] = [0x35; 32];
const MAGIC: &[u8; 8] = b"KSCAUT01";

fn message(chat_id: i32, id: i64, text: &str) -> SavedSecretMessage {
    SavedSecretMessage {
        id,
        chat_id,
        sender_id: 8,
        sender_name: "Audit sender".into(),
        outgoing: false,
        date_unix: 1_700_000_000,
        text: text.into(),
        reply_to_msg_id: None,
        ttl_seconds: None,
        media: Some(SavedMediaRecord {
            kind: SecretMediaKind::Document,
            mime_type: "application/octet-stream".into(),
            file_name: "sample.bin".into(),
            size_bytes: 5,
            local_rel_path: Some("sample.bin".into()),
            width: None,
            height: None,
            duration_secs: None,
        }),
    }
}

fn document(record: &SavedSecretMessage) -> Value {
    json!({
        "enabled": [record.chat_id],
        "messages": {
            record.chat_id.to_string(): {
                record.id.to_string(): record,
            },
        },
    })
}

fn write_authenticated(path: &Path, plaintext: &str) -> Vec<u8> {
    let mut nonce = [0; 12];
    getrandom::fill(&mut nonce).unwrap();
    let ciphertext = Aes256Gcm::new_from_slice(&KEY)
        .unwrap()
        .encrypt(
            &Nonce::from(nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: MAGIC,
            },
        )
        .unwrap();
    let mut raw = MAGIC.to_vec();
    raw.extend(nonce);
    raw.extend(ciphertext);
    fs::write(path, &raw).unwrap();
    raw
}

fn read_authenticated(path: &Path) -> String {
    let raw = fs::read(path).unwrap();
    assert_eq!(&raw[..8], MAGIC);
    let nonce: [u8; 12] = raw[8..20].try_into().unwrap();
    let plaintext = Aes256Gcm::new_from_slice(&KEY)
        .unwrap()
        .decrypt(
            &Nonce::from(nonce),
            Payload {
                msg: &raw[20..],
                aad: MAGIC,
            },
        )
        .unwrap();
    String::from_utf8(plaintext).unwrap()
}

fn assert_refused_without_loss(label: &str, plaintext: &str, marker: &str) {
    let directory = tempdir().unwrap();
    let path = directory.path().join("saved.enc");
    let before = write_authenticated(&path, plaintext);
    match AutoSaveStore::open(path.clone(), KEY) {
        Err(SecretChatError::StoreCorrupt { reason, .. }) => {
            assert!(!reason.contains(marker));
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        Err(error) => panic!("{label}: expected structural refusal, got {error:?}"),
        Ok(mut store) => {
            // Demonstrate the destructive normalization on the original code.
            // The repaired decoder refuses before this branch can rewrite it.
            store.start_auto_save(99).unwrap();
            drop(store);
            let rewritten = read_authenticated(&path);
            panic!(
                "{label}: ambiguous history was accepted and rewritten; original marker retained: {}",
                rewritten.contains(marker)
            );
        }
    }
    // A refused decode must release its writer lock for a subsequent repair.
    let valid = document(&message(7, 1, "replacement fixture"));
    write_authenticated(&path, &valid.to_string());
    assert!(AutoSaveStore::open(path, KEY).is_ok());
}

#[test]
fn duplicate_chat_identity_cannot_discard_saved_history_on_rewrite() {
    let first = serde_json::to_string(&message(7, 1, "first_history_marker")).unwrap();
    let second = serde_json::to_string(&message(7, 2, "second history")).unwrap();
    let text = format!(
        "{{\"enabled\":[7],\"messages\":{{\"7\":{{\"1\":{first}}},\"7\":{{\"2\":{second}}}}}}}"
    );
    assert_refused_without_loss("duplicate chat", &text, "first_history_marker");
}

#[test]
fn duplicate_message_identity_cannot_overwrite_saved_content_on_rewrite() {
    let first = serde_json::to_string(&message(7, 1, "first_message_marker")).unwrap();
    let second = serde_json::to_string(&message(7, 1, "replacement content")).unwrap();
    let text =
        format!("{{\"enabled\":[7],\"messages\":{{\"7\":{{\"1\":{first},\"1\":{second}}}}}}}");
    assert_refused_without_loss("duplicate message", &text, "first_message_marker");
}

#[test]
fn duplicate_enabled_chat_identities_are_not_silently_collapsed() {
    assert_refused_without_loss(
        "duplicate enabled identity",
        "{\"enabled\":[7,7],\"messages\":{}}",
        "\"enabled\":[7,7",
    );
}

#[test]
fn unknown_top_level_history_fields_are_not_silently_dropped() {
    let mut value = document(&message(7, 1, "record"));
    value["future_history"] = json!("top_level_history_marker");
    assert_refused_without_loss(
        "unknown state field",
        &value.to_string(),
        "top_level_history_marker",
    );
}

#[test]
fn unknown_message_fields_are_not_silently_dropped() {
    let mut value = document(&message(7, 1, "record"));
    value["messages"]["7"]["1"]["future_message"] = json!("message_field_marker");
    assert_refused_without_loss(
        "unknown message field",
        &value.to_string(),
        "message_field_marker",
    );
}

#[test]
fn unknown_media_fields_are_not_silently_dropped() {
    let mut value = document(&message(7, 1, "record"));
    value["messages"]["7"]["1"]["media"]["future_media"] = json!("media_field_marker");
    assert_refused_without_loss(
        "unknown media field",
        &value.to_string(),
        "media_field_marker",
    );
}

#[test]
fn existing_numeric_alias_and_index_refusals_remain_strict() {
    let record = serde_json::to_string(&message(7, 1, "record")).unwrap();
    for alias in ["07", "+7", "7.0", "7e0", "\\u0037"] {
        let text = format!("{{\"enabled\":[7],\"messages\":{{\"{alias}\":{{\"1\":{record}}}}}}}");
        assert_refused_without_loss("chat numeric alias", &text, "unlogged_marker");
    }
    for alias in ["01", "+1", "1.0", "1e0", "\\u0031"] {
        let text = format!("{{\"enabled\":[7],\"messages\":{{\"7\":{{\"{alias}\":{record}}}}}}}");
        assert_refused_without_loss("message numeric alias", &text, "unlogged_marker");
    }
    let mut wrong_chat = document(&message(7, 1, "record"));
    wrong_chat["messages"]["7"]["1"]["chat_id"] = json!(8);
    let mut wrong_message = document(&message(7, 1, "record"));
    wrong_message["messages"]["7"]["1"]["id"] = json!(2);
    for text in [
        wrong_chat.to_string(),
        wrong_message.to_string(),
        "{\"enabled\":[true],\"messages\":{}}".into(),
        "{\"enabled\":[7.0],\"messages\":{}}".into(),
        "{\"enabled\":[7],\"messages\":{\"7\":{}}}".into(),
        "{\"enabled\":[],\"enabled\":[7],\"messages\":{}}".into(),
    ] {
        assert_refused_without_loss("invalid existing structure", &text, "unlogged_marker");
    }
}

#[test]
fn legitimate_legacy_documents_and_optional_fields_survive_reopen_and_mutation() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("saved.enc");
    let mut expected = message(-7, i64::MIN, "legacy media");
    expected.media.as_mut().unwrap().local_rel_path = None;
    let mut value = document(&expected);
    let fields = value["messages"]["-7"][i64::MIN.to_string()]
        .as_object_mut()
        .unwrap();
    for field in ["reply_to_msg_id", "ttl_seconds"] {
        fields.remove(field);
    }
    let media = fields["media"].as_object_mut().unwrap();
    for field in ["local_rel_path", "width", "height", "duration_secs"] {
        media.remove(field);
    }
    write_authenticated(&path, &value.to_string());
    let mut store = AutoSaveStore::open(path.clone(), KEY).unwrap();
    assert_eq!(store.read_saved_messages(-7, 10), [expected.clone()]);
    store.start_auto_save(i32::MAX).unwrap();
    let mut second = message(i32::MAX, i64::MAX, "newly appended history");
    second.media = None;
    assert!(store.record_message(second.clone()).unwrap());
    drop(store);
    let mut reopened = AutoSaveStore::open(path.clone(), KEY).unwrap();
    assert_eq!(reopened.read_saved_messages(-7, 10), [expected]);
    assert_eq!(reopened.read_saved_messages(i32::MAX, 10), [second]);
    assert_eq!(reopened.list_saved_chats(), [-7, i32::MAX]);
    assert_eq!(reopened.delete_saved_messages(-7, None).unwrap(), 1);
    drop(reopened);
    let rewritten: Value = serde_json::from_str(&read_authenticated(&path)).unwrap();
    assert_eq!(
        rewritten
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["enabled", "messages"]
    );
    assert_eq!(rewritten["messages"].as_object().unwrap().len(), 1);
    assert!(AutoSaveStore::open(path, KEY).is_ok());
}

#[test]
fn public_record_serialization_and_documented_duplicate_insert_behavior_are_preserved() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("saved.enc");
    let original = message(7, 1, "first value remains authoritative");
    let mut public_json = serde_json::to_value(&original).unwrap();
    public_json["application_field"] = json!("caller-owned extension");
    public_json["media"]["application_field"] = json!("caller-owned media extension");
    assert_eq!(
        serde_json::from_value::<SavedSecretMessage>(public_json).unwrap(),
        original
    );
    let mut store = AutoSaveStore::open(path.clone(), KEY).unwrap();
    assert!(!store.record_message(original.clone()).unwrap());
    store.start_auto_save(7).unwrap();
    assert!(store.record_message(original.clone()).unwrap());
    let before = fs::read(&path).unwrap();
    let mut duplicate = original.clone();
    duplicate.text = "caller attempted a replacement".into();
    assert!(!store.record_message(duplicate).unwrap());
    assert_eq!(store.read_saved_messages(7, 10), [original]);
    assert_eq!(fs::read(path).unwrap(), before);
}
