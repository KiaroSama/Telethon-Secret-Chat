#![allow(clippy::unwrap_used)]
#[path = "support_live/mod.rs"]
mod live;
use kiaro_secret_chat::{
    chat::{Chat, ChatSnapshot},
    error::Result,
    manager::{Event, transport::BoxFuture},
};
use live::{LiveManager, Recorder, WaitError};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone)]
struct Fake {
    recorder: Recorder,
    closed: Arc<Mutex<Vec<i32>>>,
    accepted: bool,
}
impl LiveManager for Fake {
    fn create(
        &self,
        _: grammers_tl_types::enums::InputUser,
    ) -> BoxFuture<'_, Result<ChatSnapshot>> {
        Box::pin(async move {
            if self.accepted {
                self.recorder.record(Event::ChatReady {
                    chat_id: 7,
                    peer_user_id: 2,
                    key_fingerprint: 123,
                    key_hash: None,
                });
            }
            Ok(Chat::for_role(7, 1, 2, true).snapshot())
        })
    }
    fn close(&self, id: i32) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.closed.lock().unwrap().push(id);
            Ok(())
        })
    }
}
#[tokio::test]
async fn chat_never_accepted_is_closed() {
    let recorder = Recorder::new(2);
    recorder.record(Event::DecryptFailed {
        chat_id: 7,
        reason: "SYNTHETIC-DO-NOT-PRINT",
    });
    let manager = Fake {
        recorder: recorder.clone(),
        closed: Arc::new(Mutex::new(Vec::new())),
        accepted: false,
    };
    let failure = live::ready_chat(
        &manager,
        &recorder,
        grammers_tl_types::types::InputUser {
            user_id: 2,
            access_hash: 0,
        }
        .into(),
        |_| {},
        "t",
        Duration::from_millis(1),
    )
    .await
    .unwrap_err();
    assert_eq!(*manager.closed.lock().unwrap(), [7]);
    assert!(matches!(failure, WaitError::Timeout { .. }));
    let diagnostic = format!("{failure}{failure:?}");
    assert!(diagnostic.contains("DecryptFailed=1"));
    assert!(diagnostic.contains("interop acceptance timed out"));
    assert!(!diagnostic.contains("SYNTHETIC-DO-NOT-PRINT"));
    for id in 1..10 {
        recorder.record(Event::ChatRequested {
            chat_id: id,
            peer_user_id: 2,
        });
    }
    assert_eq!(recorder.len(), 2);
    assert_eq!(recorder.dropped(), 8);
    assert!(recorder.of("DecryptFailed").is_empty());
    assert!(!recorder.is_empty());
}
#[tokio::test]
async fn accepted_chat_is_left_open() {
    let recorder = Recorder::default();
    let manager = Fake {
        recorder: recorder.clone(),
        closed: Arc::new(Mutex::new(Vec::new())),
        accepted: true,
    };
    let (chat, event) = live::ready_chat(
        &manager,
        &recorder,
        grammers_tl_types::types::InputUser {
            user_id: 2,
            access_hash: 0,
        }
        .into(),
        |_| {},
        "t",
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(chat.id, 7);
    assert_eq!(event.chat_id(), 7);
    assert_eq!(event.name(), "ChatReady");
    assert!(manager.closed.lock().unwrap().is_empty());
    assert!(recorder.first("ChatReady", |e| e.chat_id() == 7).is_some());
    assert!(recorder.of("ChatReady").len() == 1);
}
