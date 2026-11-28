use super::support::*;
use kiaro_secret_chat::{
    custody::{CustodyLimits, CustodyStorage},
    manager::{Options, SecretChatManager},
    storage::{MemoryStorage, Storage},
};
use std::sync::{Arc, Mutex};
pub type Bounded<S> = SecretChatManager<CustodyStorage<S>, Fake>;
pub fn limits() -> CustodyLimits {
    let mut limits = CustodyLimits::new(100, 8 * 1024 * 1024, 3600.0).unwrap();
    limits.frame_bytes = 8192;
    limits
}
pub async fn pair<S: Storage + 'static>(
    storage: S,
    limits: CustodyLimits,
    options: Options,
) -> (Bounded<S>, Manager, Arc<Mutex<World>>, i32) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = SecretChatManager::new(
        aa,
        CustodyStorage::new(storage, "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        options,
    )
    .unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.stop_auto_save_secret_chats().await.unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, world, id)
}
