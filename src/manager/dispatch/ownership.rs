//! A returned blocking child remains owned even if its Tokio runtime disappears.
use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::Condvar,
    task::{Context, Poll, Wake, Waker},
};
pub(super) struct Registry {
    pub tasks: Arc<Mutex<Vec<Entry>>>,
    pub changed: Arc<Notify>,
    pub id: Id,
    pub finished: Arc<AtomicBool>,
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.finished.store(true, Ordering::Release);
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|entry| entry.id != self.id);
        }
        self.changed.notify_waiters();
    }
}
pub(super) struct Joined {
    pub result: std::result::Result<Result<()>, tokio::task::JoinError>,
    pub permit: Arc<WorkPermit>,
    pub payload: PayloadReservation,
    pub registry: Arc<Registry>,
}
struct Bundle {
    child: JoinHandle<Result<()>>,
    sender: oneshot::Sender<Joined>,
    permit: Arc<WorkPermit>,
    payload: PayloadReservation,
    registry: Arc<Registry>,
    done: Arc<AtomicBool>,
}
#[derive(Default)]
struct Slot {
    bundle: Option<Bundle>,
    closed: bool,
}
#[derive(Default)]
struct Cell {
    slot: Mutex<Slot>,
    changed: Condvar,
}
pub(super) struct Waiter {
    cell: Arc<Cell>,
}
impl Waiter {
    /// Allocate the independent owner before calling user code that can spawn.
    pub fn prepare() -> Result<Self> {
        let cell = Arc::new(Cell::default());
        let worker = cell.clone();
        std::thread::Builder::new()
            .name("secret-chat-callback".into())
            .spawn(move || {
                let bundle = {
                    let mut slot = worker
                        .slot
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
                    loop {
                        if let Some(bundle) = slot.bundle.take() {
                            break Some(bundle);
                        }
                        if slot.closed {
                            break None;
                        }
                        slot = worker
                            .changed
                            .wait(slot)
                            .unwrap_or_else(|poison| poison.into_inner());
                    }
                };
                if let Some(Bundle {
                    child,
                    sender,
                    permit,
                    payload,
                    registry,
                    done,
                }) = bundle
                {
                    let result = wait_child(child);
                    done.store(true, Ordering::Release);
                    // A closed receiver drops these resources here, after the real join.
                    let _ = sender.send(Joined {
                        result,
                        permit,
                        payload,
                        registry,
                    });
                }
            })
            .map_err(|_| Error::Capacity)?;
        Ok(Self { cell })
    }
    pub fn handoff(
        self,
        child: JoinHandle<Result<()>>,
        permit: Arc<WorkPermit>,
        payload: PayloadReservation,
        registry: Arc<Registry>,
        done: Arc<AtomicBool>,
    ) -> oneshot::Receiver<Joined> {
        let (sender, receiver) = oneshot::channel();
        self.cell
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .bundle = Some(Bundle {
            child,
            sender,
            permit,
            payload,
            registry,
            done,
        });
        self.cell.changed.notify_one();
        receiver
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        self.cell
            .slot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .closed = true;
        self.cell.changed.notify_one();
    }
}
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn wait_child(
    mut child: JoinHandle<Result<()>>,
) -> std::result::Result<Result<()>, tokio::task::JoinError> {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match Pin::new(&mut child).poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => std::thread::park(),
        }
    }
}
