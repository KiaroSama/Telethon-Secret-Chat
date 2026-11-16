use super::*;
pub(super) fn acknowledged_ids_are_budgeted() {
    let event = Event::MessageAcknowledged {
        chat_id: 7,
        seq_no: 1,
        random_ids: (0..1024).collect(),
    };
    assert!(
        event.payload_size() >= 512 + 1024 * std::mem::size_of::<i64>(),
        "each acknowledged identifier is retained by the callback clone"
    );
}
pub(super) fn callback_admission_is_all_or_nothing() {
    let budget = WorkBudget::new(None).unwrap();
    let dispatcher = crate::manager::dispatch::Dispatcher::new(budget);
    let calls = Arc::new(AtomicUsize::new(0));
    for label in ["first", "second"] {
        let seen = calls.clone();
        dispatcher
            .on("MessageAcknowledged", label, move |_| {
                seen.fetch_add(1, Ordering::AcqRel);
                Ok(())
            })
            .unwrap();
    }
    let reservations = Arc::new(AtomicUsize::new(0));
    let seen = reservations.clone();
    dispatcher
        .configure(
            1,
            false,
            |_, _| {},
            move |_, _| {
                if seen.fetch_add(1, Ordering::AcqRel) == 1 {
                    Err(Error::Capacity)
                } else {
                    Ok(PayloadReservation::empty())
                }
            },
        )
        .unwrap();
    let event = Event::MessageAcknowledged {
        chat_id: 7,
        seq_no: 1,
        random_ids: vec![1],
    };
    assert!(!dispatcher.emit(event.clone(), false, 0));
    assert_eq!(
        calls.load(Ordering::Acquire),
        0,
        "a later admission refusal must not run the earlier callback"
    );
    dispatcher
        .configure(1, false, |_, _| {}, |_, _| Ok(PayloadReservation::empty()))
        .unwrap();
    assert!(dispatcher.emit(event, false, 0));
    assert_eq!(calls.load(Ordering::Acquire), 2);
}

pub(super) async fn cleanup_panic_releases_registry() {
    let dispatcher = crate::manager::dispatch::Dispatcher::new(WorkBudget::new(None).unwrap());
    let entered = Arc::new(Notify::new());
    let mark = entered.clone();
    fn panic_cleanup(_: Event) -> transport::BoxFuture<'static, Result<()>> {
        Box::pin(async { panic!("public burned cleanup panic fixture") })
    }
    dispatcher
        .on_async_with_cleanup(
            "MessageAcknowledged",
            "panic_cleanup",
            move |_| {
                let entered = mark.clone();
                async move {
                    entered.notify_one();
                    std::future::pending::<Result<()>>().await
                }
            },
            panic_cleanup,
        )
        .unwrap();
    assert!(dispatcher.emit(
        Event::MessageAcknowledged {
            chat_id: 7,
            seq_no: 1,
            random_ids: vec![]
        },
        false,
        0
    ));
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    dispatcher.cancel_chat(7);
    assert!(
        matches!(
            tokio::time::timeout(Duration::from_secs(2), dispatcher.settle()).await,
            Ok(Ok(()))
        ),
        "a panicking cleanup must remove registry ownership and wake settle without a runtime policy"
    );
    assert_eq!(dispatcher.active(), 0);
}

pub(super) async fn fencing_waits_for_runtime_publication() {
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager = SecretChatManager::new(
        transport,
        crate::storage::MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let before = manager.inner.generation.load(Ordering::Acquire);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let publication = manager.inner.runtime_state.lock().unwrap();
    let owner = manager.clone();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        owner.fence_runtime();
        done_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let fenced = done_rx.recv_timeout(Duration::from_millis(200));
    drop(publication);
    assert!(
        fenced.is_err(),
        "runtime fencing must share the startup-publication critical section"
    );
    done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    thread.join().unwrap();
    assert!(manager.inner.stopping.load(Ordering::Acquire));
    assert_eq!(manager.inner.generation.load(Ordering::Acquire), before + 1);
}
