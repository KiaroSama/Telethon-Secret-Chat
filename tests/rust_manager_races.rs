#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    error::Error,
    manager::{Event, SendFileOptions},
    media::FileSource,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::*;
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
#[tokio::test]
async fn upload_from_prior_generation_cannot_send_or_overwrite() {
    deleting_chat_fences_upload_completion().await;
    let (a, b, world, id) = established().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().upload_gate = Some(Arc::clone(&gate));
    let old_manager = a.clone();
    let old = tokio::spawn(async move {
        old_manager
            .send_file(
                id,
                FileSource::bytes(b"payload".to_vec(), "x.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    bounded(gate.entered.notified()).await;
    a.stop().await.unwrap();
    a.start().await.unwrap();
    b.send_message(id, "message in new run", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let before = world.lock().unwrap().sent.len();
    gate.release.notify_one();
    assert!(matches!(
        bounded(old).await.unwrap(),
        Err(Error::ManagerStopping)
    ));
    assert_eq!(world.lock().unwrap().sent.len(), before);
    assert_eq!(
        a.read_history(id, 10).unwrap()[0].text,
        "message in new run"
    );
    a.send_message(id, "new run remains usable", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(
        b.read_history(id, 10).unwrap()[0].text,
        "new run remains usable"
    );
}
#[tokio::test]
async fn new_generation_does_not_wait_on_old_upload_lock() {
    let (a, b, world, id) = established().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().upload_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let old = tokio::spawn(async move {
        manager
            .send_file(
                id,
                FileSource::bytes(b"old".to_vec(), "old.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    bounded(gate.entered.notified()).await;
    a.stop().await.unwrap();
    a.start().await.unwrap();
    bounded(a.send_message(id, "new run", None, None))
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(b.read_history(id, 10).unwrap()[0].text, "new run");
    gate.release.notify_one();
    assert!(matches!(
        bounded(old).await.unwrap(),
        Err(Error::ManagerStopping)
    ));
}
async fn deleting_chat_fences_upload_completion() {
    let (a, _b, world, id) = established().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().upload_gate = Some(Arc::clone(&gate));
    let manager = a.clone();
    let old = tokio::spawn(async move {
        manager
            .send_file(
                id,
                FileSource::bytes(b"old".to_vec(), "old.bin").unwrap(),
                SendFileOptions::default(),
            )
            .await
    });
    bounded(gate.entered.notified()).await;
    a.delete_secret_chat(id).await.unwrap();
    let before = world.lock().unwrap().sent.len();
    gate.release.notify_one();
    assert!(bounded(old).await.unwrap().is_err());
    assert_eq!(world.lock().unwrap().sent.len(), before);
    assert!(matches!(a.status(id), Err(Error::UnknownChat { .. })));
}
#[tokio::test]
async fn concurrent_handlers_can_both_stop_and_finish() {
    let (a, b, world) = managers();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let completed = Arc::new(Mutex::new(Vec::new()));
    let both_entered = Arc::new(tokio::sync::Barrier::new(2));
    for n in 0..2 {
        let manager = a.clone();
        let completed = Arc::clone(&completed);
        let both_entered = Arc::clone(&both_entered);
        a.on_async("ChatReady", "stop_handler", move |_| {
            let manager = manager.clone();
            let completed = Arc::clone(&completed);
            let both_entered = Arc::clone(&both_entered);
            async move {
                both_entered.wait().await;
                manager.stop().await?;
                completed.lock().unwrap().push(n);
                Ok(())
            }
        })
        .unwrap();
    }
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    bounded(async {
        loop {
            if completed.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(a.send_message(id, "stopped", None, None).await.is_err());
    a.start().await.unwrap();
    a.send_message(id, "after concurrent stops", None, None)
        .await
        .unwrap();
}
#[tokio::test]
async fn returned_task_can_stop_and_finish_its_tail() {
    let (a, b, world) = managers();
    b.start().await.unwrap();
    let finished = Arc::new(tokio::sync::Notify::new());
    let manager = b.clone();
    let sink = Arc::clone(&finished);
    b.on_task("ChatRequested", "returned_task", move |event| {
        assert!(matches!(event, Event::ChatRequested { .. }));
        let manager = manager.clone();
        let sink = Arc::clone(&sink);
        tokio::spawn(async move {
            manager.stop().await?;
            sink.notify_one();
            Ok(())
        })
    })
    .unwrap();
    a.create(peer(2000)).await.unwrap();
    pump(&a, &b, &world).await;
    bounded(finished.notified()).await;
    b.start().await.unwrap();
}
