#![allow(clippy::unwrap_used)]
#[path = "support_manager/logging.rs"]
mod logging;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::error::{Error, Result};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::*;
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .unwrap()
}
async fn cleanup_lifecycle(start: bool, task: bool) {
    let (a, b, world, id) = established().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let sink = Arc::clone(&entered);
    let run = move |_| {
        let sink = Arc::clone(&sink);
        async move {
            sink.notify_one();
            std::future::pending::<Result<()>>().await
        }
    };
    let manager = a.clone();
    let cleaned = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&cleaned);
    let cleanup = move |_| {
        let manager = manager.clone();
        let mark = Arc::clone(&mark);
        async move {
            if start {
                assert!(matches!(manager.start().await, Err(Error::ManagerStopping)));
            } else {
                manager.stop().await?;
            }
            mark.store(true, Ordering::Release);
            Ok(())
        }
    };
    if task {
        a.on_task_with_cleanup(
            "MessageReceived",
            "cleanup",
            move |e| tokio::spawn(run(e)),
            cleanup,
        )
        .unwrap();
    } else {
        a.on_async_with_cleanup("MessageReceived", "cleanup", run, cleanup)
            .unwrap();
    }
    b.send_message(id, "run handler", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    bounded(entered.notified()).await;
    bounded(a.stop()).await.unwrap();
    assert!(cleaned.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
}
#[tokio::test]
async fn coroutine_cleanup_can_request_stop() {
    cleanup_lifecycle(false, false).await;
}
#[tokio::test]
async fn task_cleanup_can_request_stop() {
    cleanup_lifecycle(false, true).await;
}
#[tokio::test]
async fn coroutine_cleanup_cannot_restart_stopping_manager() {
    cleanup_lifecycle(true, false).await;
}
#[tokio::test]
async fn task_cleanup_cannot_restart_stopping_manager() {
    cleanup_lifecycle(true, true).await;
}
#[tokio::test]
async fn cancellation_of_one_stop_waiter_does_not_cancel_shared_cleanup() {
    cleanup_deadline_aborts_unfinished_cleanup_and_converges().await;
    let (a, b, world, id) = established().await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let run_entered = Arc::clone(&entered);
    let gate = Arc::new(Gate::default());
    let cleanup_gate = Arc::clone(&gate);
    a.on_async_with_cleanup(
        "MessageReceived",
        "wait",
        move |_| {
            let entered = Arc::clone(&run_entered);
            async move {
                entered.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let gate = Arc::clone(&cleanup_gate);
            async move {
                gate.entered.notify_one();
                gate.release.notified().await;
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(id, "start", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    bounded(entered.notified()).await;
    let manager = a.clone();
    let first = tokio::spawn(async move { manager.stop().await });
    bounded(gate.entered.notified()).await;
    let manager = a.clone();
    let second = tokio::spawn(async move { manager.stop().await });
    tokio::task::yield_now().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(!second.is_finished());
    gate.release.notify_one();
    bounded(second).await.unwrap().unwrap();
    assert_eq!(a.active_handler_tasks(), 0);
    a.start().await.unwrap();
}
#[tokio::test]
async fn start_completes_failed_shutdown_before_recovery() {
    use kiaro_secret_chat::{
        manager::{Options, SecretChatManager},
        storage::MemoryStorage,
    };
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let store = TestStorage::default();
    let failures = Arc::clone(&store.failures);
    let a = SecretChatManager::new(aa, store, "11".repeat(32), Options::default()).unwrap();
    let b = SecretChatManager::new(
        bb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    let cleaned = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&cleaned);
    a.on_async_with_cleanup(
        "MessageReceived",
        "failed_save_cleanup",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let done = Arc::clone(&done);
            async move {
                done.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    b.send_message(id, "callback alive at failed shutdown", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    bounded(entered.notified()).await;
    failures.store(1, Ordering::Release);
    assert!(matches!(a.stop().await, Err(Error::Storage)));
    assert_eq!(a.active_handler_tasks(), 0);
    assert!(cleaned.load(Ordering::Acquire));
    assert!(matches!(
        a.send_message(id, "blocked", None, None).await,
        Err(Error::ManagerStopping)
    ));
    bounded(a.start()).await.unwrap();
    a.send_message(id, "after recovered shutdown", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(
        b.read_history(id, 1).unwrap()[0].text,
        "after recovered shutdown"
    );
    a.stop().await.unwrap();
}
#[tokio::test]
async fn async_failure_is_observed_without_private_error_text() {
    let capture = logging::Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (a, b, world, id) = established().await;
    a.on_async("MessageReceived", "failing_async", |_| async {
        Err(Error::MediaKind {
            kind: "sensitive handler details".into(),
        })
    })
    .unwrap();
    b.send_message(id, "private plaintext", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    bounded(async {
        loop {
            if a.active_handler_tasks() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let log = capture.0.lock().unwrap().join("\n");
    assert!(log.contains("handler failed"));
    assert!(log.contains("failing_async"));
    assert!(!log.contains("sensitive handler details"));
    assert!(!log.contains("private plaintext"));
}
async fn discarded_failure(completed: bool, stopping: bool) {
    let capture = logging::Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (a, _b, _world) = managers();
    a.start().await.unwrap();
    if stopping {
        a.stop().await.unwrap();
    }
    let returned = if completed {
        let task = tokio::spawn(async {
            Err(Error::MediaKind {
                kind: "sensitive handler cleanup details".into(),
            })
        });
        bounded(async {
            while !task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        task
    } else {
        let task = tokio::spawn(std::future::pending::<Result<()>>());
        task.abort();
        task
    };
    let slot = Arc::new(Mutex::new(Some(returned)));
    let source = Arc::clone(&slot);
    let event = if stopping {
        "DecryptFailed"
    } else {
        "ChatRequested"
    };
    a.on_task_with_cleanup(
        event,
        "discarded",
        move |_| source.lock().unwrap().take().unwrap(),
        |_| async {
            Err(Error::MediaKind {
                kind: "sensitive handler cleanup details".into(),
            })
        },
    )
    .unwrap();
    if stopping {
        a.handle_update(
            &grammers_tl_types::types::UpdateNewEncryptedMessage {
                message: grammers_tl_types::types::EncryptedMessageService {
                    random_id: 1,
                    chat_id: 424242,
                    date: 0,
                    bytes: vec![0; 64],
                }
                .into(),
                qts: 0,
            }
            .into(),
        )
        .await;
    } else {
        use kiaro_secret_chat::dh::{DhConfig, SecretExponent};
        let dh = DhConfig::new(2, &prime()).unwrap();
        a.handle_encryption(
            grammers_tl_types::types::EncryptedChatRequested {
                folder_id: None,
                id: 7,
                access_hash: 88,
                date: 1,
                admin_id: 2000,
                participant_id: 1000,
                g_a: dh.public_value(&SecretExponent::generate().unwrap()),
            }
            .into(),
        )
        .await
        .unwrap();
    }
    bounded(async {
        loop {
            if a.active_handler_tasks() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let log = capture.0.lock().unwrap().join("\n");
    assert!(log.contains("handler"));
    assert!(!log.contains("sensitive handler cleanup details"));
    assert!(slot.lock().unwrap().is_none());
}
#[tokio::test]
async fn completed_failure_is_retrieved_during_running() {
    discarded_failure(true, false).await;
}
#[tokio::test]
async fn completed_failure_is_retrieved_during_stopping() {
    discarded_failure(true, true).await;
}
#[tokio::test]
async fn cancelled_task_cleanup_failure_is_retrieved_during_running() {
    discarded_failure(false, false).await;
}
#[tokio::test]
async fn cancelled_task_cleanup_failure_is_retrieved_during_stopping() {
    discarded_failure(false, true).await;
}
#[tokio::test]
async fn discarded_task_cleanup_cannot_restart_after_stopping() {
    let (a, _b, _world) = managers();
    a.start().await.unwrap();
    a.stop().await.unwrap();
    let checked = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&checked);
    let manager = a.clone();
    a.on_task_with_cleanup(
        "DecryptFailed",
        "discarded_restart",
        |_| tokio::spawn(std::future::pending::<Result<()>>()),
        move |_| {
            let manager = manager.clone();
            let mark = Arc::clone(&mark);
            async move {
                assert!(matches!(manager.start().await, Err(Error::ManagerStopping)));
                mark.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    a.handle_update(
        &grammers_tl_types::types::UpdateNewEncryptedMessage {
            message: grammers_tl_types::types::EncryptedMessageService {
                random_id: 1,
                chat_id: 424242,
                date: 0,
                bytes: vec![0; 64],
            }
            .into(),
            qts: 0,
        }
        .into(),
    )
    .await;
    bounded(async {
        while a.active_handler_tasks() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(checked.load(Ordering::Acquire));
}
async fn cleanup_deadline_aborts_unfinished_cleanup_and_converges() {
    use kiaro_secret_chat::{
        manager::{Options, SecretChatManager},
        storage::MemoryStorage,
    };
    let (aa, bb) = Fake::pair();
    let world = Arc::clone(&aa.world);
    let a = SecretChatManager::new(
        aa,
        MemoryStorage::default(),
        "11".repeat(32),
        Options {
            handler_cleanup_timeout: Duration::from_millis(40),
            ..Default::default()
        },
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
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = Arc::clone(&entered);
    a.on_async_with_cleanup(
        "MessageReceived",
        "never_finishes",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        |_| std::future::pending::<Result<()>>(),
    )
    .unwrap();
    b.send_message(id, "start", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    bounded(entered.notified()).await;
    bounded(a.stop()).await.unwrap();
    assert_eq!(a.active_handler_tasks(), 0);
    bounded(a.start()).await.unwrap();
    a.send_message(id, "resumed", None, None).await.unwrap();
}
