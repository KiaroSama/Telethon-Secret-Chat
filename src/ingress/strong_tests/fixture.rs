use super::*;
use crate::{
    chat::{Chat, ChatState, DhRecord},
    crypto::{Direction, SharedKey, encrypt_frame},
    custody::{CustodyLimits, CustodyStorage, IngressLimits},
    enums,
    manager::{Options, SecretChatManager, StrongSecretChatManager},
    storage::Storage,
    types,
};
use grammers_mtsender::SenderPool;
use grammers_session::{SessionData, storages::MemorySession, updates::UpdatesLike};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
pub(super) type Strong = StrongSecretChatManager<fake::Fake>;
pub(super) struct Fixture {
    pub manager: Strong,
    pub pool: SenderPool,
    pub ingress: Arc<DurableIngress>,
    pub world: Arc<Mutex<fake::World>>,
    pub dir: tempfile::TempDir,
}
pub(super) fn budget() -> CustodyLimits {
    let mut limits = CustodyLimits::new(32, 1024 * 1024, 3600.0).unwrap();
    limits.frame_bytes = 8192;
    limits
}
pub(super) fn pool() -> SenderPool {
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some(std::array::from_fn(|i| i as u8));
    SenderPool::new(Arc::new(MemorySession::from(data)), 123)
}
pub(super) async fn manager(ingress: &Arc<DurableIngress>) -> (Strong, Arc<Mutex<fake::World>>) {
    let (client, _) = fake::Fake::pair();
    let world = Arc::clone(&client.world);
    let auth = ingress.binding().unwrap().digest();
    for id in ingress.core.lock().unwrap().state.chats.keys() {
        world
            .lock()
            .unwrap()
            .requests
            .insert(*id, (1000, 2000, vec![]));
    }
    let storage =
        CustodyStorage::new(ingress.protocol_storage().unwrap(), auth.clone(), budget()).unwrap();
    let base = SecretChatManager::new(client, storage, auth, Options::default()).unwrap();
    let manager = Strong::test_from_manager(base, Arc::clone(ingress)).unwrap();
    manager.start().await.unwrap();
    (manager, world)
}
pub(super) async fn started() -> Fixture {
    let (dir, ingress, pool) = super::super::test_fixture::fixture(64).await;
    let (manager, world) = manager(&ingress).await;
    Fixture {
        manager,
        pool,
        ingress,
        world,
        dir,
    }
}
pub(super) fn chat(id: i32) -> Chat {
    let mut chat = Chat::new(id, 88, 2000, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: fake::prime(),
    });
    chat.capability_layer = 144;
    chat.rekeyed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    chat
}
pub(super) fn install(f: &Fixture, id: i32) {
    f.manager.test_install_chat(chat(id)).unwrap();
    f.world
        .lock()
        .unwrap()
        .requests
        .insert(id, (1000, 2000, vec![]));
}
pub(super) fn frame(sequence: i32, identity: i64, text: &str) -> Vec<u8> {
    let message = types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: identity,
        ttl: 0,
        message: text.into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    };
    encrypt(message.into(), sequence)
}
pub(super) fn encrypt(message: enums::DecryptedMessage, sequence: i32) -> Vec<u8> {
    encrypt_frame(
        &SharedKey::from_slice(&[7; 256]).unwrap(),
        &crate::framing::wrap(message, 144, 1, sequence * 2)
            .unwrap()
            .to_bytes(),
        Direction::Responder,
    )
    .unwrap()
}
pub(super) fn update(id: i32, qts: i32, identity: i64, bytes: Vec<u8>) -> tl::enums::Update {
    tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessage {
            random_id: identity,
            chat_id: id,
            date: 124,
            bytes,
            file: tl::enums::EncryptedFile::Empty,
        }
        .into(),
        qts,
    }
    .into()
}
pub(super) fn envelope(updates: Vec<tl::enums::Update>, seq: i32) -> UpdatesLike {
    UpdatesLike::Updates(
        tl::types::Updates {
            updates,
            users: vec![],
            chats: vec![],
            date: 124,
            seq,
        }
        .into(),
    )
}
pub(super) async fn capture(f: &Fixture, updates: Vec<tl::enums::Update>, seq: i32) {
    let (tx, mut receiver) = super::super::test_fixture::receiver(&f.ingress, &f.pool).await;
    super::super::test_fixture::capture(&mut receiver, &tx, envelope(updates, seq)).await;
}
pub(super) fn message(receipt: &crate::custody::RelayDelivery) -> types::DecryptedMessageLayer {
    crate::framing::unwrap(&receipt.message.body).unwrap()
}
pub(super) async fn reopen(
    path: &Path,
    binding: crate::binding::AuthorizationBinding,
) -> (Strong, Arc<DurableIngress>, SenderPool) {
    let storage = EncryptedIngressStorage::open(path, [9; 32], binding, 1024 * 1024).unwrap();
    let ingress = DurableIngress::new(
        storage,
        IngressLimits::new(64, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let pool = pool();
    ingress.prepare(&pool.handle, None).await.unwrap();
    // This is the same burned receipt/protocol fixture proof as test_fixture.rs.
    // Successful native activation is tested separately by the original SDK harness.
    ingress.core.lock().unwrap().verified = true;
    let (manager, _) = manager(&ingress).await;
    (manager, ingress, pool)
}
pub(super) fn release_after_crash(ingress: &Arc<DurableIngress>) {
    ingress.fence().unwrap();
    let mut core = ingress.core.lock().unwrap();
    core.backend.suspend_storage().unwrap();
    core.released = true;
}
pub(super) async fn requested(f: &Fixture, id: i32) {
    let dh = crate::dh::DhConfig::new(2, &fake::prime()).unwrap();
    let secret = crate::dh::SecretExponent::generate().unwrap();
    capture(
        f,
        vec![
            tl::types::UpdateEncryption {
                chat: tl::types::EncryptedChatRequested {
                    folder_id: None,
                    id,
                    access_hash: 88,
                    date: 124,
                    admin_id: 2000,
                    participant_id: 123,
                    g_a: dh.public_value(&secret),
                }
                .into(),
                date: 124,
            }
            .into(),
        ],
        2,
    )
    .await;
}
