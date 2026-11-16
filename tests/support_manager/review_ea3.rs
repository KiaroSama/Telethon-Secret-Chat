use super::*;
pub(super) async fn late_token_cannot_outlive_fence() {
    type Manager = SecretChatManager<
        crate::custody::TransientStorage<crate::storage::MemoryStorage>,
        crate::test_support::support::Fake,
    >;
    let gate: Arc<Mutex<Option<Manager>>> = Arc::default();
    let hook = gate.clone();
    let options = Options {
        admission: Some(Arc::new(move |_, _| {
            if let Some(manager) = hook.lock().unwrap().take() {
                manager.fence_runtime();
            }
            true
        })),
        ..Default::default()
    };
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager = SecretChatManager::new(
        transport,
        crate::custody::TransientStorage::new(
            crate::storage::MemoryStorage::default(),
            "11".repeat(32),
            crate::custody::TransientLimits::default(),
        )
        .unwrap(),
        "11".repeat(32),
        options,
    )
    .unwrap();
    manager.start().await.unwrap();
    manager
        .protocol(|p| {
            p.insert_chat(crate::chat::Chat::new(
                7,
                8,
                2000,
                false,
                crate::chat::ChatState::Waiting,
            ))
        })
        .unwrap();
    *gate.lock().unwrap() = Some(manager.clone());
    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let result = manager
        .with_payload_operation(Some(7), WorkKind::Ordinary, 0, async move {
            observed.store(true, Ordering::Release);
            Ok(())
        })
        .await;
    let stopped_before_poll = matches!(result, Err(Error::ManagerStopping));
    manager.stop().await.unwrap();
    assert!(
        stopped_before_poll && !polled.load(Ordering::Acquire),
        "a token registered after the runtime fence must not permit one poll of owned work"
    );
}

struct StartStorage {
    inner: crate::storage::MemoryStorage,
    fail_commit: Arc<AtomicBool>,
    finish_hook: Arc<Mutex<Option<SecretChatManager<Self, crate::test_support::support::Fake>>>>,
    hook: Arc<Mutex<Option<SecretChatManager<Self, crate::test_support::support::Fake>>>>,
}
impl crate::storage::Storage for StartStorage {
    fn load(&self) -> Result<Option<crate::storage::State>> {
        if let Some(manager) = self.hook.lock().unwrap().take() {
            manager.fence_runtime();
        }
        self.inner.load()
    }
    fn commit(&mut self, state: &crate::storage::State) -> Result<()> {
        if self.fail_commit.swap(false, Ordering::AcqRel) {
            return Err(Error::Storage);
        }
        self.inner.commit(state)
    }
    fn suspend_storage(&mut self) -> Result<()> {
        if let Some(manager) = self.finish_hook.lock().unwrap().take() {
            manager.fence_runtime();
        }
        Ok(())
    }
    fn resume_storage(&mut self) -> Result<Option<crate::storage::State>> {
        self.load()
    }
}
pub(super) async fn startup_cannot_recapture_a_new_fence() {
    stale_start_cannot_reacquire_released_writer().await;
    startup_retry_cannot_adopt_an_unowned_fence().await;
    let hook = Arc::default();
    let storage = StartStorage {
        inner: Default::default(),
        hook: Arc::clone(&hook),
        fail_commit: Arc::default(),
        finish_hook: Arc::default(),
    };
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    manager.stop().await.unwrap();
    *hook.lock().unwrap() = Some(manager.clone());
    let before = manager.inner.generation.load(Ordering::Acquire);
    let result = manager.start().await;
    assert!(
        manager.inner.generation.load(Ordering::Acquire) > before,
        "resume hook must actually fence startup"
    );
    let refused = matches!(result, Err(Error::ManagerStopping));
    let published = manager.inner.running.load(Ordering::Acquire);
    manager.stop().await.unwrap();
    assert!(
        refused && !published,
        "startup must retain the generation from before its stopping check and storage reload"
    );
    manager.start().await.unwrap();
    manager.stop().await.unwrap();
}

pub(super) async fn blocked_child_keeps_ownership() {
    let policy = crate::storage::RuntimePolicy {
        operation_seconds: 2.0,
        cleanup_seconds: 0.02,
        tasks: 1,
        bytes: 65536,
        chats: 8,
        records: 16,
        frame_bytes: 4096,
    };
    let budget = WorkBudget::new(Some(policy)).unwrap();
    let dispatcher = crate::manager::dispatch::Dispatcher::new(budget.clone());
    let released = Arc::new(AtomicBool::new(false));
    let marker = released.clone();
    dispatcher
        .configure(
            1,
            false,
            |_, _| {},
            move |_, _| {
                let marker = marker.clone();
                Ok(PayloadReservation::new(move || {
                    marker.store(true, Ordering::Release)
                }))
            },
        )
        .unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let factory = Arc::new(Mutex::new(Some((entered_tx, release_rx, done_tx))));
    dispatcher
        .on_task("MessageAcknowledged", "blocked_child", move |_| {
            let (entered, release, done) = factory.lock().unwrap().take().unwrap();
            tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                release.recv().unwrap();
                done.send(()).unwrap();
                Ok(())
            })
        })
        .unwrap();
    assert!(dispatcher.emit(
        Event::MessageAcknowledged {
            chat_id: 7,
            seq_no: 1,
            random_ids: vec![1]
        },
        false,
        0
    ));
    entered_rx.await.unwrap();
    let refusal = dispatcher
        .drain(&std::collections::HashSet::new(), Duration::from_millis(20))
        .await;
    let refused = refusal.is_err();
    let counted = dispatcher.active() == 1
        && budget.active.load(Ordering::Acquire) == 1
        && !released.load(Ordering::Acquire);
    release_tx.send(()).unwrap();
    done_rx.await.unwrap();
    dispatcher.settle().await.unwrap();
    assert!(
        refused && counted,
        "a started spawn_blocking child must remain registered, budgeted and payload-owned after bounded drain"
    );
    assert_eq!(dispatcher.active(), 0);
    assert!(released.load(Ordering::Acquire));
}

pub(super) async fn factory_admission_is_part_of_drain() {
    let dispatcher = crate::manager::dispatch::Dispatcher::new(WorkBudget::new(None).unwrap());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let factory = Arc::new(Mutex::new(Some((entered_tx, release_rx))));
    dispatcher
        .on_task("MessageAcknowledged", "returning_child", move |_| {
            let (entered, release) = factory.lock().unwrap().take().unwrap();
            let child = tokio::spawn(std::future::pending::<Result<()>>());
            entered.send(()).unwrap();
            release.recv().unwrap();
            child
        })
        .unwrap();
    let runtime = tokio::runtime::Handle::current();
    let owner = dispatcher.clone();
    let emit = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        owner.emit(
            Event::MessageAcknowledged {
                chat_id: 7,
                seq_no: 1,
                random_ids: vec![1],
            },
            false,
            0,
        )
    });
    entered_rx.await.unwrap();
    let escaped = tokio::time::timeout(
        Duration::from_millis(20),
        dispatcher.drain(&std::collections::HashSet::new(), Duration::from_secs(1)),
    )
    .await
    .is_ok();
    release_tx.send(()).unwrap();
    emit.join().unwrap();
    dispatcher.cancel_chat(7);
    tokio::time::timeout(Duration::from_secs(2), dispatcher.settle())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !escaped,
        "drain must include a callback whose already-spawned child has not yet been returned for registration"
    );
}

pub(super) async fn runtime_loss_keeps_blocking_child_budgeted() {
    for asynchronous in [false, true] {
        runtime_loss_callback(asynchronous).await;
    }
}
async fn runtime_loss_callback(asynchronous: bool) {
    let budget = WorkBudget::new(Some(crate::storage::RuntimePolicy {
        operation_seconds: 2.0,
        cleanup_seconds: 0.02,
        tasks: 1,
        bytes: 65536,
        chats: 8,
        records: 16,
        frame_bytes: 4096,
    }))
    .unwrap();
    let dispatcher = crate::manager::dispatch::Dispatcher::new(budget.clone());
    let released = Arc::new(AtomicBool::new(false));
    let marker = released.clone();
    dispatcher
        .configure(
            1,
            false,
            |_, _| {},
            move |_, _| {
                let marker = marker.clone();
                Ok(PayloadReservation::new(move || {
                    marker.store(true, Ordering::Release)
                }))
            },
        )
        .unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let factory = Arc::new(Mutex::new(Some((release_rx, entered_tx))));
    if asynchronous {
        dispatcher
            .on_async("MessageAcknowledged", "runtime_loss_async", move |_| {
                let (release, entered) = factory.lock().unwrap().take().unwrap();
                async move {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                    Ok(())
                }
            })
            .unwrap();
    } else {
        dispatcher
            .on_task("MessageAcknowledged", "runtime_loss", move |_| {
                let (release, entered) = factory.lock().unwrap().take().unwrap();
                tokio::task::spawn_blocking(move || {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                    Ok(())
                })
            })
            .unwrap();
    }
    let owner = dispatcher.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            assert!(owner.emit(
                Event::MessageAcknowledged {
                    chat_id: 7,
                    seq_no: 1,
                    random_ids: vec![1]
                },
                false,
                0
            ));
            entered_rx.await.unwrap();
        });
        runtime.shutdown_timeout(Duration::ZERO);
    })
    .join()
    .unwrap();
    let counted = dispatcher.active() == 1
        && budget.active.load(Ordering::Acquire) == 1
        && !released.load(Ordering::Acquire);
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), dispatcher.settle())
        .await
        .unwrap()
        .unwrap();
    assert!(
        counted,
        "destroying the callback runtime must not erase ownership of a running blocking child"
    );
    assert!(released.load(Ordering::Acquire));
    assert_eq!(budget.active.load(Ordering::Acquire), 0);
}

pub(super) async fn refused_service_callback_retains_delivery() {
    let (a, b, world, id) = crate::test_support::support::established().await;
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    a.on("ServiceActionReceived", "retained_service", move |_| {
        seen.fetch_add(1, Ordering::AcqRel);
        Ok(())
    })
    .unwrap();
    a.inner.dispatch.fence();
    b.set_ttl(id, 42).await.unwrap();
    let update = world
        .lock()
        .unwrap()
        .updates
        .get_mut(&1000)
        .unwrap()
        .pop_front()
        .unwrap();
    let grammers_tl_types::enums::Update::NewEncryptedMessage(update) = update else {
        panic!("encrypted TTL service")
    };
    let grammers_tl_types::enums::EncryptedMessage::Service(message) = update.message else {
        panic!("TTL service frame")
    };
    let result = a.receive(id, &message.bytes, None, message.date).await;
    assert!(matches!(result, Err(Error::Capacity)));
    let pending = a.protocol(|p| p.pending_service_deliveries(id)).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.applied, Some(true));
    assert_eq!(count.load(Ordering::Acquire), 0);
    a.inner.dispatch.resume().unwrap();
    a.drain_deliveries(id).await.unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);
    assert!(
        a.protocol(|p| p.pending_service_deliveries(id))
            .unwrap()
            .is_empty()
    );
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

pub(super) async fn unfinished_callback_retains_store_writer() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("owned.json");
    let storage = crate::storage::FileStorage::open(&path).unwrap();
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager = SecretChatManager::new(
        transport,
        storage,
        "11".repeat(32),
        Options {
            handler_cleanup_timeout: Duration::from_millis(20),
            ..Default::default()
        },
    )
    .unwrap();
    manager.start().await.unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let factory = Arc::new(Mutex::new(Some((entered_tx, release_rx))));
    manager
        .on_task("MessageAcknowledged", "writer_owned", move |_| {
            let (entered, release) = factory.lock().unwrap().take().unwrap();
            tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                release.recv().unwrap();
                Ok(())
            })
        })
        .unwrap();
    assert!(manager.emit(Event::MessageAcknowledged {
        chat_id: 7,
        seq_no: 1,
        random_ids: vec![1]
    }));
    entered_rx.await.unwrap();
    let result = manager.stop().await;
    let writer_held = crate::storage::FileStorage::open(&path).is_err();
    release_tx.send(()).unwrap();
    manager.inner.dispatch.settle().await.unwrap();
    manager.stop().await.unwrap();
    let replacement = crate::storage::FileStorage::open(&path).unwrap();
    drop(replacement);
    assert!(
        matches!(result, Err(Error::ManagerStopping)) && writer_held,
        "a bounded callback join failure must retain the store writer until its real child finishes"
    );
}

pub(super) async fn returning_child_can_stop_without_self_abort() {
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager = SecretChatManager::new(
        transport,
        crate::storage::MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let factory = Arc::new(Mutex::new(Some((done_tx, started_tx, release_rx))));
    let stopper = manager.clone();
    manager
        .on_task("MessageAcknowledged", "self_stop_return", move |_| {
            let (done, started, release) = factory.lock().unwrap().take().unwrap();
            let manager = stopper.clone();
            let child = tokio::spawn(async move {
                started.send(()).unwrap();
                manager.stop().await?;
                done.send(()).unwrap();
                Ok(())
            });
            release.recv().unwrap();
            child
        })
        .unwrap();
    let runtime = tokio::runtime::Handle::current();
    let emitter = manager.clone();
    let emit = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        emitter.emit(Event::MessageAcknowledged {
            chat_id: 7,
            seq_no: 1,
            random_ids: vec![1],
        })
    });
    started_rx.await.unwrap();
    assert!(manager.inner.stopping.load(Ordering::Acquire));
    release_tx.send(()).unwrap();
    emit.join().unwrap();
    tokio::time::timeout(Duration::from_secs(2), done_rx)
        .await
        .unwrap()
        .unwrap();
    manager.inner.dispatch.settle().await.unwrap();
}

async fn startup_retry_cannot_adopt_an_unowned_fence() {
    let fail_commit = Arc::new(AtomicBool::new(false));
    let finish_hook = Arc::default();
    let storage = StartStorage {
        inner: Default::default(),
        hook: Arc::default(),
        fail_commit: fail_commit.clone(),
        finish_hook: Arc::clone(&finish_hook),
    };
    let (transport, _) = crate::test_support::support::Fake::pair();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    manager.start().await.unwrap();
    fail_commit.store(true, Ordering::Release);
    assert!(matches!(manager.stop().await, Err(Error::Storage)));
    *finish_hook.lock().unwrap() = Some(manager.clone());
    let before = manager.inner.generation.load(Ordering::Acquire);
    let result = manager.start().await;
    let advanced = manager.inner.generation.load(Ordering::Acquire) >= before + 2;
    let published = manager.inner.running.load(Ordering::Acquire);
    manager.stop().await.unwrap();
    assert!(
        advanced,
        "retry must include both its owned fence and the independent completion-time fence"
    );
    assert!(
        matches!(result, Err(Error::ManagerStopping)) && !published,
        "a successful retry must not adopt an unrelated fence after its own stop generation"
    );
    manager.start().await.unwrap();
    manager.stop().await.unwrap();
}

async fn stale_start_cannot_reacquire_released_writer() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("restart.json");
    let storage = crate::storage::FileStorage::open(&path).unwrap();
    let (transport, _) = crate::test_support::support::Fake::pair();
    let world = transport.world.clone();
    let manager =
        SecretChatManager::new(transport, storage, "11".repeat(32), Options::default()).unwrap();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    world.lock().unwrap().resume_hook = Some(Box::new(move || {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    }));
    let runtime = tokio::runtime::Handle::current();
    let starter = manager.clone();
    let start = std::thread::spawn(move || runtime.block_on(starter.start()));
    entered_rx.await.unwrap();
    manager.stop().await.unwrap();
    let available = crate::storage::FileStorage::open(&path).unwrap();
    drop(available);
    release_tx.send(()).unwrap();
    let result = start.join().unwrap();
    let released = crate::storage::FileStorage::open(&path);
    let remained_released = released.is_ok();
    drop(released);
    manager.stop().await.unwrap();
    assert!(
        matches!(result, Err(Error::ManagerStopping)) && remained_released,
        "a stale start must not reacquire the writer after a completed external stop"
    );
}
