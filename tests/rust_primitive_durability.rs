#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::{Chat, ChatState, DhRecord},
    crypto::SharedKey,
    files::{self, FileKey},
    manager::{Options, SecretChatManager},
    storage::{FileStorage, MemoryStorage, State, Storage},
    wire,
};
fn exact_size(size: usize) {
    let key = FileKey::generate().unwrap();
    let data = vec![b'x'; size];
    let mut encrypted = Vec::new();
    files::encrypt_file(&mut &data[..], &mut encrypted, size as u64, &key).unwrap();
    let mut result = Vec::new();
    files::decrypt_file(&mut &encrypted[..], &mut result, size as u64, &key).unwrap();
    assert_eq!(result, data);
    assert_eq!(encrypted.len(), size.div_ceil(16) * 16);
}
#[test]
fn length0() {
    exact_size(0);
}
#[test]
fn length1() {
    exact_size(1);
}
#[test]
fn length15() {
    exact_size(15);
}
#[test]
fn length16() {
    exact_size(16);
}
#[test]
fn length17() {
    exact_size(17);
}
#[test]
fn length255() {
    exact_size(255);
}
#[test]
fn length256() {
    exact_size(256);
}
#[test]
fn length257() {
    exact_size(257);
}
#[test]
fn malformed_ciphertext_never_aborts_process() {
    const ENV: &str = "TSC_PRIMITIVE_CIPHER_CHILD";
    if std::env::var_os(ENV).is_some() {
        let key = FileKey::new(&[b'k'; 32], &[b'i'; 32]).unwrap();
        assert!(files::decrypt_file(&mut &[b'x'; 17][..], &mut Vec::new(), 1, &key).is_err());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "malformed_ciphertext_never_aborts_process",
            "--nocapture",
        ])
        .env(ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child aborted or accepted malformed ciphertext: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn impossible(count: i32) {
    let mut bytes = 0x0c4f40beu32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&0x1cb5c415u32.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    assert!(wire::validate(&bytes, "DecryptedMessageAction").is_err());
}
#[test]
fn impossible_vector_negative() {
    impossible(-1);
}
#[test]
fn impossible_vector_huge() {
    impossible(i32::MAX);
}
#[test]
fn invalid_utf8_is_not_repaired() {
    use kiaro_secret_chat::{Serializable, enums, types};
    let mut bytes = enums::DecryptedMessage::from(types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1,
        ttl: 0,
        message: "x".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    })
    .to_bytes();
    let n = bytes.len();
    bytes[n - 4..].copy_from_slice(&[1, 255, 0, 0]);
    assert!(wire::validate(&bytes, "DecryptedMessage").is_err());
}
#[tokio::test]
async fn file_storage_close_removes_live_key_material() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("private.json");
    let raw: [u8; 256] = std::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(3));
    let mut chat = Chat::new(42, 88, 2000, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&raw).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: support::prime(),
    });
    chat.capability_layer = 144;
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(42, chat);
    let mut storage = FileStorage::open(&path).unwrap();
    storage.commit(&state).unwrap();
    let (a, _) = support::Fake::pair();
    a.world
        .lock()
        .unwrap()
        .requests
        .insert(42, (1000, 2000, vec![0; 256]));
    let manager = SecretChatManager::new(a, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    manager
        .send_message(42, "delete stored plaintext", None, None)
        .await
        .unwrap();
    manager.close(42).await.unwrap();
    manager.stop().await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&hex::encode(raw)));
    assert!(!text.contains(&hex::encode("delete stored plaintext")));
    let back = FileStorage::open(&path).unwrap().load().unwrap().unwrap();
    assert!(back.chats[&42].key.is_none());
    let _ = MemoryStorage::default();
}
