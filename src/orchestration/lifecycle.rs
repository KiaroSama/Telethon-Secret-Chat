use std::{collections::VecDeque, num::NonZeroUsize, sync::Mutex};
use tokio::sync::Notify;

use crate::{errors::SecretChatError, media::parameter};

/// A lifecycle notice with no message, key, or custody payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    ChatRequested { chat_id: i32 },
    ChatReady { chat_id: i32 },
    ChatPaused { chat_id: i32 },
    ChatResumed { chat_id: i32 },
    ChatClosed { chat_id: i32 },
}

/// What a producer does when the subscriber's queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    DropOldest,
    DropNewest,
    Refuse,
}

/// An event the producer still owns. Refused events do not count as drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LifecyclePublishError {
    #[error("lifecycle subscription is full")]
    Full(LifecycleEvent),
    #[error("lifecycle subscription is closed")]
    Closed(LifecycleEvent),
}

#[derive(Default)]
struct Pending {
    queue: VecDeque<LifecycleEvent>,
    closed: bool,
    dropped: u64,
}

/// An opt-in queue for caller-published notices, for example from a manager callback.
/// Managers create no queue by default. The caller owns the queue and can share it
/// with producers using `Arc`.
pub struct BoundedLifecycleSubscription {
    capacity: NonZeroUsize,
    policy: BackpressurePolicy,
    pending: Mutex<Pending>,
    changed: Notify,
}

impl BoundedLifecycleSubscription {
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> Result<Self, SecretChatError> {
        Ok(Self {
            capacity: NonZeroUsize::new(capacity)
                .ok_or_else(|| parameter("lifecycle subscription capacity must be positive"))?,
            policy,
            pending: Mutex::default(),
            changed: Notify::new(),
        })
    }

    /// Offer a notice without waiting for a receiver or consuming any manager task slots.
    pub fn publish(&self, event: LifecycleEvent) -> Result<(), LifecyclePublishError> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.closed {
            return Err(LifecyclePublishError::Closed(event));
        }
        if pending.queue.len() == self.capacity.get() {
            match self.policy {
                BackpressurePolicy::Refuse => return Err(LifecyclePublishError::Full(event)),
                BackpressurePolicy::DropNewest => {
                    pending.dropped = pending.dropped.saturating_add(1);
                    return Ok(());
                }
                BackpressurePolicy::DropOldest => {
                    pending.queue.pop_front();
                    pending.dropped = pending.dropped.saturating_add(1);
                }
            }
        }
        pending.queue.push_back(event);
        drop(pending);
        self.changed.notify_one();
        Ok(())
    }

    /// Publish the lifecycle notices exposed by typed manager callbacks. Other events are ignored.
    pub fn publish_manager_event(
        &self,
        event: &crate::manager::Event,
    ) -> Result<bool, LifecyclePublishError> {
        use crate::manager::Event;
        let event = match *event {
            Event::ChatRequested { chat_id, .. } => LifecycleEvent::ChatRequested { chat_id },
            Event::ChatReady { chat_id, .. } => LifecycleEvent::ChatReady { chat_id },
            Event::ChatClosed { chat_id, .. } => LifecycleEvent::ChatClosed { chat_id },
            _ => return Ok(false),
        };
        self.publish(event).map(|()| true)
    }

    /// Wait for the next notice, or return `None` once the closed queue is empty.
    pub async fn recv(&self) -> Option<LifecycleEvent> {
        loop {
            let mut changed = std::pin::pin!(self.changed.notified());
            changed.as_mut().enable();
            {
                let mut pending = self
                    .pending
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(event) = pending.queue.pop_front() {
                    return Some(event);
                }
                if pending.closed {
                    return None;
                }
            }
            changed.await;
        }
    }

    /// Count losses caused by the configured overflow policy.
    pub fn dropped_count(&self) -> u64 {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dropped
    }

    /// Stop accepting notices and let receivers drain the backlog.
    pub fn close(&self) {
        self.finish(false);
    }

    /// Stop accepting notices and discard the backlog immediately.
    pub fn unsubscribe(&self) {
        self.finish(true);
    }

    fn finish(&self, discard: bool) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.closed = true;
        if discard {
            pending.queue = VecDeque::new();
        }
        drop(pending);
        self.changed.notify_waiters();
    }
}
