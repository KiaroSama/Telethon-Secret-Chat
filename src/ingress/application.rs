//! Task-local application identity. User callback tasks do not inherit it.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone)]
pub(super) struct Application {
    pub owner: u64,
    pub batch: u64,
    pub index: usize,
    pub hash: [u8; 32],
    pub kind: Kind,
    pub chat_id: i32,
    eligible: Arc<AtomicBool>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Message,
    Encryption,
    Terminal,
    Other,
}
impl Application {
    pub fn new(
        owner: u64,
        batch: u64,
        index: usize,
        hash: [u8; 32],
        kind: Kind,
        chat_id: i32,
    ) -> Self {
        Self {
            owner,
            batch,
            index,
            hash,
            kind,
            chat_id,
            eligible: Arc::new(AtomicBool::new(false)),
        }
    }
}
tokio::task_local! {static CURRENT:Application;}
pub(super) async fn scope<F: Future>(application: Application, future: F) -> F::Output {
    CURRENT.scope(application, future).await
}
pub(super) fn current(owner: u64) -> Option<Application> {
    CURRENT
        .try_with(|app| {
            (app.owner == owner && app.eligible.load(Ordering::Acquire)).then(|| app.clone())
        })
        .ok()
        .flatten()
}
pub(crate) fn transition<T>(kind: Kind, chat_id: i32, work: impl FnOnce() -> T) -> T {
    let flag = CURRENT
        .try_with(|app| {
            (app.chat_id == chat_id && (app.kind == kind || kind == Kind::Terminal))
                .then(|| Arc::clone(&app.eligible))
        })
        .ok()
        .flatten();
    if let Some(flag) = &flag {
        flag.store(true, Ordering::Release);
    }
    struct Reset(Option<Arc<AtomicBool>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            if let Some(flag) = &self.0 {
                flag.store(false, Ordering::Release);
            }
        }
    }
    let _reset = Reset(flag);
    work()
}
