//! Invalid attachment cannot mutate an existing caller-owned SDK update path.
#![allow(clippy::unwrap_used)]
use super::test_fixture::*;
use grammers_mtsender::{UpdatesConfiguration, UpdatesReceiver};
#[tokio::test]
async fn missing_initial_baseline_refuses_without_attachment() {
    let (dir, ingress, pool, _baseline) = unprepared(32);
    let path = dir.path().join("state/custody.bin");
    let before = std::fs::read(&path).unwrap();
    assert!(ingress.prepare(&pool.handle, None).await.is_err());
    assert!(!ingress.core.lock().unwrap().attached);
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[tokio::test]
async fn running_original_pool_refuses_without_committing_baseline() {
    let (dir, ingress, pool, baseline) = unprepared(32);
    let path = dir.path().join("state/custody.bin");
    let before = std::fs::read(&path).unwrap();
    let mut run = Box::pin(pool.runner.run());
    std::future::poll_fn(|cx| {
        assert!(run.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(ingress.prepare(&pool.handle, Some(baseline)).await.is_err());
    assert!(!ingress.core.lock().unwrap().attached);
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[tokio::test]
async fn replacing_reconstructed_baseline_refuses_without_attachment() {
    let (dir, ingress, pool, baseline) = unprepared(32);
    let path = dir.path().join("state/custody.bin");
    {
        let mut core = ingress.core.lock().unwrap();
        let mut ledger = core.ledger.clone();
        ledger.bootstrap(baseline.clone()).unwrap();
        let state = core.state.clone();
        core.persist(state, ledger).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    assert!(ingress.prepare(&pool.handle, Some(baseline)).await.is_err());
    assert!(!ingress.core.lock().unwrap().attached);
    assert_eq!(std::fs::read(path).unwrap(), before);
}
#[tokio::test]
async fn occupied_native_update_seam_refuses_without_conversion() {
    let (dir, ingress, pool, baseline) = unprepared(32);
    let path = dir.path().join("state/custody.bin");
    let before = std::fs::read(&path).unwrap();
    let _receiver = UpdatesReceiver::create(
        pool.handle.thin.clone(),
        pool.handle.session.clone(),
        pool.updates,
        UpdatesConfiguration::default(),
    )
    .await
    .unwrap();
    assert!(ingress.prepare(&pool.handle, Some(baseline)).await.is_err());
    assert!(!ingress.core.lock().unwrap().attached);
    assert_eq!(std::fs::read(path).unwrap(), before);
}
