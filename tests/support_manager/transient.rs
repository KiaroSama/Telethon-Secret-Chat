use super::support::*;
use kiaro_secret_chat::{
    custody::{TransientLimits, TransientStorage},
    manager::{Options, SecretChatManager},
    storage::{MemoryStorage, Storage},
};
use std::sync::{Arc, Mutex};
pub type Transient<S> = SecretChatManager<TransientStorage<S>, Fake>;
pub async fn pair<S: Storage + 'static>(
    storage: S,
    limits: TransientLimits,
) -> (Transient<S>, Manager, Arc<Mutex<World>>, i32) {
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = SecretChatManager::new(
        aa,
        TransientStorage::new(storage, "11".repeat(32), limits).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
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
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, world, id)
}
