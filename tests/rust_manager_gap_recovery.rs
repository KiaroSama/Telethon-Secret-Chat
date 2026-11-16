#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    chat::ChatState,
    manager::{Event, RpcFailure},
};
use std::sync::{Arc, Mutex};
use support::*;
async fn lost_resend(late_first: bool) {
    let (a, b, w, id) = established().await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    a.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message.text);
        }
        Ok(())
    })
    .unwrap();
    w.lock()
        .unwrap()
        .fail_sends_from
        .insert(1000, RpcFailure::Transient);
    assert!(
        a.send_message(id, "left pending by failed send", None, None)
            .await
            .is_err()
    );
    b.send_message(id, "m0", None, None).await.unwrap();
    let first = w
        .lock()
        .unwrap()
        .updates
        .get_mut(&1000)
        .unwrap()
        .pop_front()
        .unwrap();
    if late_first {
        b.send_message(id, "m1", None, None).await.unwrap();
        let _ = w
            .lock()
            .unwrap()
            .updates
            .get_mut(&1000)
            .unwrap()
            .pop_front()
            .unwrap();
        b.send_message(id, "m2", None, None).await.unwrap();
        pump(&a, &b, &w).await;
        a.handle_update(&first).await;
        assert_eq!(*got.lock().unwrap(), vec!["m0"]);
    } else {
        b.send_message(id, "m1", None, None).await.unwrap();
        pump(&a, &b, &w).await;
        assert!(got.lock().unwrap().is_empty());
    }
    w.lock().unwrap().fail_sends_from.remove(&1000);
    if late_first {
        a.send_message(id, "x", None, None).await.unwrap();
        pump(&a, &b, &w).await;
        b.send_message(id, "m3", None, None).await.unwrap();
    } else {
        b.send_message(id, "m2", None, None).await.unwrap();
    }
    pump(&a, &b, &w).await;
    assert_eq!(
        *got.lock().unwrap(),
        if late_first {
            vec!["m0", "m1", "m2", "m3"]
        } else {
            vec!["m0", "m1", "m2"]
        }
    );
    assert_eq!(b.status(id).unwrap().state, ChatState::Ready);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
#[tokio::test]
async fn failed_gap_request_is_retried_after_next_incoming_message() {
    lost_resend(false).await;
}
#[tokio::test]
async fn retried_gap_request_does_not_end_peer_after_missing_prefix_arrives() {
    lost_resend(true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_incoming_updates_deliver_in_sequence_order() {
    let (a, b, w, id) = established().await;
    let got = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&got);
    b.on("MessageReceived", "collect", move |event| {
        if let Event::MessageReceived(message) = event {
            seen.lock().unwrap().push(message.text);
        }
        Ok(())
    })
    .unwrap();
    for text in ["one", "two", "three"] {
        a.send_message(id, text, None, None).await.unwrap();
    }
    let updates = w.lock().unwrap().updates.remove(&2000).unwrap();
    let tasks: Vec<_> = updates
        .into_iter()
        .map(|update| {
            let manager = b.clone();
            tokio::spawn(async move {
                manager.handle_update(&update).await;
            })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    pump(&a, &b, &w).await;
    assert_eq!(*got.lock().unwrap(), vec!["one", "two", "three"]);
    assert_eq!(b.status(id).unwrap().state, ChatState::Ready);
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
