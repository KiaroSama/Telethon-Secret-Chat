//! Bounded native interop observation and acceptance cleanup. No account is opened here.
use kiaro_secret_chat::{
    Serializable,
    chat::{ChatSnapshot, ChatState},
    manager::{Event, SecretChatManager, Transport, transport::BoxFuture},
    storage::Storage,
};
use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
pub struct Recorder {
    inner: Arc<Inner>,
}
struct Inner {
    state: Mutex<Recorded>,
    changed: tokio::sync::Notify,
    limit: usize,
}
#[derive(Default)]
struct Recorded {
    events: VecDeque<(usize, Event)>,
    bytes: usize,
    dropped: u64,
}
const BYTE_LIMIT: usize = 16 * 1024 * 1024;
fn kind(name: &str) -> Option<&'static str> {
    match name {
        "ChatRequested" => Some("ChatRequested"),
        "ChatReady" => Some("ChatReady"),
        "ChatClosed" | "ChatClosedEvent" => Some("ChatClosedEvent"),
        "MessageReceived" => Some("MessageReceived"),
        "MessageAcknowledged" => Some("MessageAcknowledged"),
        "ServiceActionReceived" => Some("ServiceActionReceived"),
        "DecryptFailed" => Some("DecryptFailed"),
        "SendFailed" => Some("SendFailed"),
        _ => None,
    }
}
fn size(event: &Event) -> usize {
    let dynamic = match event {
        Event::MessageReceived(m) => m
            .text
            .len()
            .saturating_add(
                m.entities
                    .as_ref()
                    .map_or(0, |e| e.iter().map(|e| e.to_bytes().len()).sum()),
            )
            .saturating_add(m.media.as_ref().map_or(0, |m| m.to_bytes().len()))
            .saturating_add(
                m.media_reference
                    .as_ref()
                    .map_or(0, |m| m.media_bytes().len()),
            ),
        Event::ServiceActionReceived {
            action,
            action_name,
            ..
        } => action.to_bytes().len().saturating_add(action_name.len()),
        Event::ChatReady { key_hash, .. } => key_hash.as_ref().map_or(0, Vec::len),
        _ => 0,
    };
    dynamic.saturating_add(512)
}
impl Default for Recorder {
    fn default() -> Self {
        Self::new(8192)
    }
}
impl Recorder {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(Recorded::default()),
                changed: tokio::sync::Notify::new(),
                limit: limit.clamp(1, 65_536),
            }),
        }
    }
    pub fn record(&self, event: Event) {
        let bytes = size(&event);
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        if bytes > BYTE_LIMIT {
            state.dropped = state.dropped.saturating_add(1);
        } else {
            while state.events.len() >= self.inner.limit
                || state.bytes.saturating_add(bytes) > BYTE_LIMIT
            {
                let Some((bytes, _)) = state.events.pop_front() else {
                    break;
                };
                state.bytes = state.bytes.saturating_sub(bytes);
                state.dropped = state.dropped.saturating_add(1);
            }
            state.bytes = state.bytes.saturating_add(bytes);
            state.events.push_back((bytes, event));
        }
        drop(state);
        self.inner.changed.notify_waiters();
    }
    pub fn first(&self, name: &str, predicate: impl Fn(&Event) -> bool) -> Option<Event> {
        let kind = kind(name)?;
        let state = self.inner.state.lock().ok()?;
        state
            .events
            .iter()
            .find(|(_, e)| e.name() == kind && predicate(e))
            .map(|(_, e)| e.clone())
    }
    pub fn of(&self, name: &str) -> Vec<Event> {
        let Some(kind) = kind(name) else {
            return Vec::new();
        };
        let Ok(state) = self.inner.state.lock() else {
            return Vec::new();
        };
        state
            .events
            .iter()
            .filter(|(_, e)| e.name() == kind)
            .map(|(_, e)| e.clone())
            .collect()
    }
    pub fn len(&self) -> usize {
        self.inner.state.lock().map_or(0, |s| s.events.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn dropped(&self) -> u64 {
        self.inner.state.lock().map_or(0, |s| s.dropped)
    }
    fn timeout(&self, kind: &'static str) -> WaitError {
        let mut observed = BTreeMap::new();
        let mut dropped = 0;
        if let Ok(state) = self.inner.state.lock() {
            dropped = state.dropped;
            for (_, e) in &state.events {
                *observed.entry(e.name()).or_insert(0usize) += 1;
            }
        }
        WaitError::Timeout {
            kind,
            observed,
            dropped,
        }
    }
    pub async fn wait_for(
        &self,
        name: &'static str,
        deadline: Duration,
        predicate: impl Fn(&Event) -> bool,
    ) -> Result<Event, WaitError> {
        let kind = kind(name).ok_or(WaitError::UnknownKind)?;
        let until = tokio::time::Instant::now()
            .checked_add(deadline)
            .ok_or(WaitError::Deadline)?;
        loop {
            let changed = self.inner.changed.notified();
            if let Some(event) = self.first(kind, &predicate) {
                return Ok(event);
            }
            if tokio::time::timeout_at(until, changed).await.is_err() {
                return Err(self.timeout(kind));
            }
        }
    }
}
#[derive(Clone)]
pub enum WaitError {
    Creation,
    Cleanup,
    UnknownKind,
    Deadline,
    Timeout {
        kind: &'static str,
        observed: BTreeMap<&'static str, usize>,
        dropped: u64,
    },
}
impl fmt::Display for WaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Creation => f.write_str("interop chat creation failed"),
            Self::Cleanup => f.write_str("interop acceptance cleanup failed"),
            Self::UnknownKind => f.write_str("unknown interop event kind"),
            Self::Deadline => f.write_str("invalid interop wait deadline"),
            Self::Timeout {
                kind,
                observed,
                dropped,
            } => {
                write!(
                    f,
                    "interop acceptance timed out waiting for {kind}; event counts:"
                )?;
                for (name, count) in observed {
                    write!(f, " {name}={count}")?;
                }
                if observed.is_empty() {
                    f.write_str(" nothing")?;
                }
                write!(f, "; dropped={dropped}")
            }
        }
    }
}
impl fmt::Debug for WaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for WaitError {}
pub trait LiveManager: Clone + Send + Sync + 'static {
    fn create(
        &self,
        peer: grammers_tl_types::enums::InputUser,
    ) -> BoxFuture<'_, kiaro_secret_chat::error::Result<ChatSnapshot>>;
    fn close(&self, id: i32) -> BoxFuture<'_, kiaro_secret_chat::error::Result<()>>;
}
impl<S: Storage + 'static, T: Transport> LiveManager for SecretChatManager<S, T> {
    fn create(
        &self,
        peer: grammers_tl_types::enums::InputUser,
    ) -> BoxFuture<'_, kiaro_secret_chat::error::Result<ChatSnapshot>> {
        Box::pin(SecretChatManager::create(self, peer))
    }
    fn close(&self, id: i32) -> BoxFuture<'_, kiaro_secret_chat::error::Result<()>> {
        Box::pin(SecretChatManager::close(self, id))
    }
}
struct Acceptance<M: LiveManager> {
    manager: M,
    id: i32,
    armed: bool,
}
impl<M: LiveManager> Drop for Acceptance<M> {
    fn drop(&mut self) {
        if self.armed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let manager = self.manager.clone();
            let id = self.id;
            runtime.spawn(async move {
                let _ = manager.close(id).await;
            });
        }
    }
}
pub async fn ready_chat<M: LiveManager>(
    manager: &M,
    recorder: &Recorder,
    peer: grammers_tl_types::enums::InputUser,
    announce: impl Fn(&str),
    label: &str,
    deadline: Duration,
) -> Result<(ChatSnapshot, Event), WaitError> {
    let mut chat = manager
        .create(peer)
        .await
        .map_err(|_| WaitError::Creation)?;
    let mut pending = Acceptance {
        manager: manager.clone(),
        id: chat.id,
        armed: true,
    };
    announce(&format!(
        "Keep the expected peer device online to accept secret chat ({label}); waiting at most {} seconds.",
        deadline.as_secs()
    ));
    let result = recorder
        .wait_for("ChatReady", deadline, |event| event.chat_id() == chat.id)
        .await;
    match result {
        Ok(ready) => {
            pending.armed = false;
            if let Event::ChatReady {
                key_fingerprint,
                key_hash,
                ..
            } = &ready
            {
                chat.state = ChatState::Ready;
                chat.key_fingerprint = Some(*key_fingerprint);
                chat.key_hash = key_hash.clone();
            }
            Ok((chat, ready))
        }
        Err(error) => {
            manager
                .close(chat.id)
                .await
                .map_err(|_| WaitError::Cleanup)?;
            pending.armed = false;
            Err(error)
        }
    }
}
