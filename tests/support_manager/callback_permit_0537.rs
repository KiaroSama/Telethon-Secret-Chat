use super::*;

type Manager = SecretChatManager<TransientStorage<support::TestStorage>, support::Fake>;
type Gates = Arc<
    Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
>;
struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Cancel,
    Deadline,
    Cleanup,
}

pub(super) async fn run() {
    for mode in [Mode::Cancel, Mode::Deadline, Mode::Cleanup] {
        run_one(mode).await;
    }
}
async fn blocking_operation(
    manager: Manager,
    io: Arc<crate::media::blocking::IoTracker>,
    gates: Gates,
    abandon: bool,
) -> Result<()> {
    let (entered, release) = gates.lock().unwrap().take().unwrap();
    manager
        .with_payload_operation(
            None,
            WorkKind::Ordinary,
            1024,
            io.scope(async move {
                let worker = crate::media::blocking::spawn(move |_| {
                    let _ = entered.send(());
                    release.recv().map_err(|_| Error::Storage)
                })?;
                if abandon {
                    tokio::pin!(worker);
                    std::future::poll_fn(|cx| {
                        assert!(std::future::Future::poll(worker.as_mut(), cx).is_pending());
                        std::task::Poll::Ready(())
                    })
                    .await;
                    return Err(Error::Paused);
                }
                worker.await??;
                Ok(())
            }),
        )
        .await
}
async fn run_one(mode: Mode) {
    let (transport, _) = support::Fake::pair();
    let manager = SecretChatManager::new(
        transport,
        TransientStorage::new(
            support::TestStorage::default(),
            "11".repeat(32),
            TransientLimits {
                bytes: 65536,
                frame_bytes: 4096,
                tasks: 1,
                operation_seconds: 0.2,
                cleanup_seconds: 0.2,
                ..Default::default()
            },
        )
        .unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    let budget = manager.inner.work.clone();
    let dispatcher = crate::manager::dispatch::Dispatcher::new(budget.clone());
    dispatcher
        .configure(
            Arc::as_ptr(&manager.inner) as usize,
            false,
            |_, _| {},
            |_, _| Ok(PayloadReservation::empty()),
        )
        .unwrap();
    let io = Arc::new(crate::media::blocking::IoTracker::default());
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Release(Some(release_tx));
    let gates = Arc::new(Mutex::new(Some((entered_tx, release_rx))));
    let callback_manager = manager.clone();
    let callback_io = io.clone();
    let began = Arc::new(Notify::new());
    let started = began.clone();
    if matches!(mode, Mode::Cleanup) {
        dispatcher
            .on_async_with_cleanup(
                "MessageAcknowledged",
                "blocking_cleanup",
                move |_| {
                    let started = started.clone();
                    async move {
                        started.notify_one();
                        std::future::pending::<Result<()>>().await
                    }
                },
                move |_| {
                    blocking_operation(
                        callback_manager.clone(),
                        callback_io.clone(),
                        gates.clone(),
                        true,
                    )
                },
            )
            .unwrap();
    } else {
        dispatcher
            .on_async("MessageAcknowledged", "blocking_callback", move |_| {
                blocking_operation(
                    callback_manager.clone(),
                    callback_io.clone(),
                    gates.clone(),
                    false,
                )
            })
            .unwrap();
    }
    assert!(dispatcher.emit(
        Event::MessageAcknowledged {
            chat_id: 7,
            seq_no: 0,
            random_ids: vec![1]
        },
        false,
        manager.generation().unwrap()
    ));
    if matches!(mode, Mode::Cleanup) {
        tokio::time::timeout(Duration::from_secs(2), began.notified())
            .await
            .unwrap();
        dispatcher.cancel_chat(7);
    }
    tokio::time::timeout(Duration::from_secs(2), entered_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(budget.acquire(1).is_err());
    if matches!(mode, Mode::Cancel) {
        dispatcher.cancel_chat(7);
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while dispatcher.active() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let retained = budget.acquire(1).is_err();
    drop(release);
    tokio::time::timeout(Duration::from_secs(2), io.drain())
        .await
        .unwrap()
        .unwrap();
    drop(budget.acquire(1).unwrap());
    manager.stop().await.unwrap();
    assert!(
        retained,
        "callback cancellation, deadline or cleanup released its sole permit while its actual blocking worker still ran"
    );
}
