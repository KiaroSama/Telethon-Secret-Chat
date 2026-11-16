//! Managed secret chats sharing the caller's Telegram authorization and update pump.
mod conversation;
mod custody;
mod dispatch;
mod establishment;
mod events;
mod expiry;
mod export;
mod lifecycle;
mod media;
mod receiving;
mod runtime;
mod saved_downloads;
mod strong;
pub mod transport;

use crate::{
    autosave::AutoSave,
    chat::{Chat, ChatSnapshot},
    error::{Error, Result},
    protocol::Protocol,
    storage::Storage,
};
pub use custody::{BoundedSecretChatManager, TransientSecretChatManager};
pub use events::{Event, MessageReceived};
pub use media::SendFileOptions;
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
pub use strong::StrongSecretChatManager;
use tokio::{
    sync::{Mutex as AsyncMutex, Notify},
    task::{Id, JoinHandle},
};
pub use transport::{PermanentError, RpcFailure, Sent, Transport};
/// Synchronous finite-mode admission; false refuses before the public effect.
pub type Admission = Arc<dyn Fn(Option<i32>, crate::storage::Operation) -> bool + Send + Sync>;
#[derive(Clone)]
pub struct Options {
    pub history_limit: usize,
    pub auto_save_folder: Option<PathBuf>,
    pub handler_cleanup_timeout: Duration,
    pub admission: Option<Admission>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            history_limit: 1000,
            auto_save_folder: None,
            handler_cleanup_timeout: Duration::from_secs(5),
            admission: None,
        }
    }
}
struct Shutdown {
    generation: u64,
    result: Mutex<Option<Result<()>>>,
    notify: Notify,
}
impl Shutdown {
    async fn wait(&self) -> Result<()> {
        loop {
            let notified = self.notify.notified();
            if let Some(result) = self.result.lock().map_err(|_| Error::Storage)?.clone() {
                return result;
            }
            notified.await;
        }
    }
}
struct Inner<S: Storage, T: Transport> {
    protocol: Mutex<Protocol<S>>,
    transport: T,
    options: Options,
    running: AtomicBool,
    startup_complete: AtomicBool,
    binding_generation: AtomicU64,
    stopping: AtomicBool,
    generation: AtomicU64,
    start_lock: AsyncMutex<()>,
    runtime_state: Mutex<()>,
    shutdown: Mutex<Option<Arc<Shutdown>>>,
    stop_callers: Mutex<HashSet<Id>>,
    outbound: Mutex<BTreeMap<i32, Arc<AsyncMutex<()>>>>,
    wire: Mutex<BTreeMap<i32, Arc<AsyncMutex<()>>>>,
    epochs: Mutex<BTreeMap<i32, u64>>,
    creating: AtomicUsize,
    creation_generation: AtomicU64,
    accepting_requests: AtomicBool,
    diagnosed: Mutex<HashSet<i32>>,
    expiry: Mutex<Option<JoinHandle<()>>>,
    expiry_changed: Arc<Notify>,
    creation_cancel: Mutex<Arc<runtime::Cancellation>>,
    creation_slots: Mutex<BTreeMap<u64, Option<String>>>,
    headroom: Mutex<BTreeMap<(i32, u64), runtime::Headroom>>,
    early: Mutex<BTreeMap<i32, grammers_tl_types::enums::EncryptedChat>>,
    dispatch: dispatch::Dispatcher,
    history: Mutex<BTreeMap<i32, VecDeque<MessageReceived>>>,
    delivering: Mutex<BTreeMap<i32, (Arc<()>, bool)>>,
    delivered: Mutex<HashSet<(i32, i32)>>,
    pending_acknowledgements: Mutex<HashSet<(i32, i32, bool)>>,
    autosave: Mutex<AutoSave>,
    downloads: saved_downloads::SavedDownloads,
    archive_io: Arc<crate::media::blocking::IoTracker>,
    work: Arc<runtime::WorkBudget>,
    reservation: AtomicU64,
    cancellations: Mutex<BTreeMap<i32, Arc<runtime::Cancellation>>>,
}
/// A clonable handle. Network waits never hold the protocol/storage mutex; an
/// upload keeps only its chat's outbound-order lock so incoming messages proceed.
pub struct SecretChatManager<S: Storage + 'static, T: Transport> {
    inner: Arc<Inner<S, T>>,
}
impl<S: Storage + 'static, T: Transport> Clone for SecretChatManager<S, T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    /// Build without creating a client or sending any request. Storage is explicit.
    pub fn new(
        transport: T,
        storage: S,
        authorization: String,
        mut options: Options,
    ) -> Result<Self> {
        if options.handler_cleanup_timeout.is_zero()
            || options.handler_cleanup_timeout > Duration::from_secs(30)
        {
            return Err(Error::Capacity);
        }
        let mut protocol = Protocol::open(storage, authorization)?;
        let _ = protocol.start()?;
        if protocol.mode() != crate::storage::StoreMode::Durable {
            options.history_limit = 0;
        }
        let mode = protocol.mode();
        let work = runtime::WorkBudget::new(protocol.runtime_policy())?;
        let epochs = protocol
            .chat_records()
            .into_iter()
            .map(|chat| (chat.id, 0))
            .collect();
        let manager = Self {
            inner: Arc::new(Inner {
                protocol: Mutex::new(protocol),
                transport,
                options,
                running: AtomicBool::new(false),
                startup_complete: AtomicBool::new(false),
                binding_generation: AtomicU64::new(u64::MAX),
                stopping: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                start_lock: AsyncMutex::new(()),
                runtime_state: Mutex::new(()),
                shutdown: Mutex::new(None),
                stop_callers: Mutex::new(HashSet::new()),
                outbound: Mutex::new(BTreeMap::new()),
                wire: Mutex::new(BTreeMap::new()),
                epochs: Mutex::new(epochs),
                creating: AtomicUsize::new(0),
                creation_generation: AtomicU64::new(0),
                accepting_requests: AtomicBool::new(true),
                diagnosed: Mutex::default(),
                expiry: Mutex::new(None),
                expiry_changed: Arc::default(),
                creation_cancel: Mutex::new(Arc::default()),
                creation_slots: Mutex::default(),
                headroom: Mutex::default(),
                early: Mutex::new(BTreeMap::new()),
                dispatch: dispatch::Dispatcher::new(Arc::clone(&work)),
                history: Mutex::new(BTreeMap::new()),
                delivering: Mutex::default(),
                delivered: Mutex::new(HashSet::new()),
                pending_acknowledgements: Mutex::default(),
                autosave: Mutex::new(AutoSave::default()),
                downloads: Default::default(),
                archive_io: Arc::default(),
                work,
                reservation: AtomicU64::new(1),
                cancellations: Mutex::new(BTreeMap::new()),
            }),
        };
        let weak = Arc::downgrade(&manager.inner);
        let payload_owner = weak.clone();
        manager.inner.dispatch.configure(
            Arc::as_ptr(&manager.inner) as usize,
            mode == crate::storage::StoreMode::Protected,
            move |id, generation| {
                if let Some(inner) = weak.upgrade()
                    && inner.generation.load(Ordering::Acquire) == generation
                {
                    Self { inner }.runtime_refuse(
                        Some(id),
                        crate::storage::RuntimeRefusal::Callback,
                        false,
                    );
                }
            },
            move |id, amount| {
                let inner = payload_owner.upgrade().ok_or(Error::ManagerStopping)?;
                Self { inner }.reserve_payload(Some(id), amount)
            },
        )?;
        Ok(manager)
    }
    /// Derive the binding from a verified same-client transport before constructing.
    pub async fn verified(transport: T, storage: S, options: Options) -> Result<Self> {
        let digest = transport
            .authorization_digest()
            .await?
            .ok_or(Error::Authorization)?;
        Self::new(transport, storage, digest, options)
    }
    async fn ensure_binding(&self) -> Result<()> {
        let generation = self.inner.generation.load(Ordering::Acquire);
        if self.inner.binding_generation.load(Ordering::Acquire) == generation {
            return Ok(());
        }
        let actual = self.inner.transport.authorization_digest().await?;
        if let Some(actual) = actual {
            let expected = self.protocol(|p| Ok(p.authorization_digest().to_owned()))?;
            if actual != expected {
                return Err(Error::Authorization);
            }
        } else if self.inner.transport.requires_verified_binding() {
            return Err(Error::Authorization);
        }
        if self.inner.generation.load(Ordering::Acquire) != generation {
            return Err(Error::ManagerStopping);
        }
        self.inner
            .binding_generation
            .store(generation, Ordering::Release);
        Ok(())
    }
    fn protocol<R>(&self, work: impl FnOnce(&mut Protocol<S>) -> Result<R>) -> Result<R> {
        work(&mut *self.inner.protocol.lock().map_err(|_| Error::Storage)?)
    }
    fn generation(&self) -> Result<u64> {
        let generation = self.inner.generation.load(Ordering::Acquire);
        self.check_generation(generation)?;
        Ok(generation)
    }
    fn check_generation(&self, generation: u64) -> Result<()> {
        if self.inner.stopping.load(Ordering::Acquire)
            || self.inner.generation.load(Ordering::Acquire) != generation
        {
            return Err(Error::ManagerStopping);
        }
        Ok(())
    }
    fn witness(&self, id: i32) -> Result<(u64, u64)> {
        let generation = self.generation()?;
        self.require_chat(id)?;
        let epoch = self
            .inner
            .epochs
            .lock()
            .map_err(|_| Error::Storage)?
            .get(&id)
            .copied()
            .unwrap_or(0);
        Ok((generation, epoch))
    }
    fn check_witness(&self, id: i32, witness: (u64, u64)) -> Result<()> {
        self.check_generation(witness.0)?;
        let epoch = self
            .inner
            .epochs
            .lock()
            .map_err(|_| Error::Storage)?
            .get(&id)
            .copied()
            .unwrap_or(0);
        if epoch != witness.1 {
            return Err(Error::ManagerStopping);
        }
        self.require_chat(id)?.require_ready()
    }
    fn bump_epoch(&self, id: i32) -> Result<()> {
        let mut epochs = self.inner.epochs.lock().map_err(|_| Error::Storage)?;
        let epoch = epochs.entry(id).or_default();
        *epoch = epoch.wrapping_add(1);
        Ok(())
    }
    fn outbound(&self, id: i32) -> Result<Arc<AsyncMutex<()>>> {
        let mut locks = self.inner.outbound.lock().map_err(|_| Error::Storage)?;
        Ok(Arc::clone(
            locks
                .entry(id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        ))
    }
    fn require_chat(&self, id: i32) -> Result<Chat> {
        self.protocol(|protocol| {
            protocol
                .chat_record(id)
                .map_err(|_| Error::UnknownChat { chat_id: id })
        })
    }
    fn sendable(&self, id: i32) -> Result<Chat> {
        self.generation()?;
        let chat = self.require_chat(id)?;
        match chat.state {
            crate::chat::ChatState::Closed => return Err(Error::ChatClosed { chat_id: id }),
            crate::chat::ChatState::Requested | crate::chat::ChatState::Waiting => {
                return Err(Error::ChatNotReady { chat_id: id });
            }
            _ => {}
        }
        chat.require_ready()?;
        Ok(chat)
    }
    /// Register a synchronous Result-returning handler. It runs before emit returns.
    pub fn on(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> Result<()> + Send + Sync + 'static,
    ) -> Result<()> {
        if self.protocol(|p| Ok(p.mode()))? == crate::storage::StoreMode::Protected {
            return Err(Error::MediaField { field: "handler" });
        }
        self.inner.dispatch.on(event, name, handler)
    }
    /// Register an async handler; the manager owns and observes its spawned task.
    pub fn on_async<F, Fut>(&self, event: &str, name: &'static str, handler: F) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner.dispatch.on_async(event, name, handler)
    }
    /// Register a handler that returns an existing Tokio task, retaining ownership.
    pub fn on_task(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        self.inner.dispatch.on_task(event, name, handler)
    }
    /// Run an owned async cleanup after cancellation; Rust future Drop cannot await.
    pub fn on_async_with_cleanup<F, Fut, C, Cut>(
        &self,
        event: &str,
        name: &'static str,
        handler: F,
        cleanup: C,
    ) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner
            .dispatch
            .on_async_with_cleanup(event, name, handler, cleanup)
    }
    /// Attach cancellation cleanup to an externally created handler task.
    pub fn on_task_with_cleanup<C, Cut>(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
        cleanup: C,
    ) -> Result<()>
    where
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner
            .dispatch
            .on_task_with_cleanup(event, name, handler, cleanup)
    }
    fn emit(&self, event: Event) -> bool {
        let id = event.chat_id();
        let admitted = self.inner.dispatch.emit(
            event,
            self.inner.stopping.load(Ordering::Acquire),
            self.inner.generation.load(Ordering::Acquire),
        );
        if !admitted
            && self
                .protocol(|p| Ok(p.mode()))
                .is_ok_and(|mode| mode == crate::storage::StoreMode::Protected)
        {
            self.runtime_refuse(Some(id), crate::storage::RuntimeRefusal::Capacity, false);
        }
        admitted
    }
    /// Detached snapshots contain no shared keys or retained ciphertext.
    pub fn list(&self) -> Result<Vec<ChatSnapshot>> {
        self.protocol(|protocol| Ok(protocol.snapshots()))
    }
    /// Inspect one detached conversation snapshot.
    pub fn status(&self, id: i32) -> Result<ChatSnapshot> {
        Ok(self.require_chat(id)?.snapshot())
    }
    /// Read the most recent received messages, oldest first; this cache is volatile.
    pub fn read_history(&self, id: i32, limit: usize) -> Result<Vec<MessageReceived>> {
        self.require_chat(id)?;
        let history = self.inner.history.lock().map_err(|_| Error::Storage)?;
        Ok(history
            .get(&id)
            .map(|items| {
                items
                    .iter()
                    .skip(items.len().saturating_sub(limit))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }
    /// Number of callback tasks still owned by this manager.
    pub fn active_handler_tasks(&self) -> usize {
        self.inner.dispatch.active()
    }
    /// Await callbacks already owned at entry; finite modes bound wait and cleanup.
    pub async fn settle(&self) -> Result<()> {
        self.inner.dispatch.settle().await
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
