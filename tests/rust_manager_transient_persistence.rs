#![allow(clippy::unwrap_used)]
#[path = "support_manager/logging.rs"]
mod logging;
#[path = "support_manager/mod.rs"]
mod support;
#[path = "support_manager/transient.rs"]
mod transient;
use kiaro_secret_chat::{
    custody::{ProtectedFileStorage, TransientLimits},
    error::Result,
    manager::{Event, SendFileOptions},
    media::FileSource,
    storage::{State, Storage},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use support::*;
use tracing::instrument::WithSubscriber;
use transient::*;
fn collector<S: Storage + 'static>(
    manager: &Transient<S>,
) -> Arc<Mutex<Vec<kiaro_secret_chat::manager::MessageReceived>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&events);
    manager
        .on_async("MessageReceived", "collect", move |event| {
            let seen = Arc::clone(&seen);
            async move {
                if let Event::MessageReceived(message) = event {
                    seen.lock().unwrap().push(*message);
                }
                Ok(())
            }
        })
        .unwrap();
    events
}
#[tokio::test]
async fn transient_send_receive_is_absent_from_real_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.json");
    let (a, b, w, id) = pair(
        ProtectedFileStorage::open(&path).unwrap(),
        TransientLimits::default(),
    )
    .await;
    let events = collector(&a);
    let text = "synthetic private text not to persist";
    a.send_message(id, text, None, None).await.unwrap();
    pump(&a, &b, &w).await;
    b.send_message(id, text, None, None).await.unwrap();
    pump(&a, &b, &w).await;
    a.settle().await.unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
    assert_eq!(events.lock().unwrap()[0].text, text);
    assert!(!std::fs::read_to_string(&path).unwrap().contains(text));
    assert!(!a.is_suspended(id).unwrap());
    a.stop().await.unwrap();
    assert!(a.is_suspended(id).unwrap());
    assert!(!std::fs::read_to_string(&path).unwrap().contains(text));
    assert!(w.lock().unwrap().discards.is_empty());
    b.stop().await.unwrap();
}
#[tokio::test]
async fn transient_media_stays_in_ram_and_never_writes_key_or_iv() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("protected.json");
    let (a, b, w, id) = pair(
        ProtectedFileStorage::open(&path).unwrap(),
        TransientLimits::default(),
    )
    .await;
    let events = collector(&a);
    let content = b"synthetic file body".repeat(4);
    b.send_file(
        id,
        FileSource::bytes(content.clone(), "sample.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &w).await;
    a.settle().await.unwrap();
    let message = events.lock().unwrap()[0].clone();
    let reference = message.media_reference.as_ref().unwrap();
    let key = reference.validate().unwrap().key;
    let mut before: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    before.sort();
    assert_eq!(a.receive_message_file(&message).await.unwrap(), content);
    let mut after: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    after.sort();
    assert_eq!(before, after);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(&hex::encode(key.key())));
    assert!(!raw.contains(&hex::encode(key.iv())));
    assert_eq!(w.lock().unwrap().downloads, 1);
    assert!(a.auto_save_secret_chats().unwrap().is_none());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}
struct Snapshots {
    backend: ProtectedFileStorage,
    path: PathBuf,
    rows: Arc<Mutex<Vec<Vec<u8>>>>,
}
impl Storage for Snapshots {
    fn load(&self) -> Result<Option<State>> {
        self.backend.load()
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        self.backend.commit(state)?;
        self.rows
            .lock()
            .unwrap()
            .push(std::fs::read(&self.path).unwrap());
        Ok(())
    }
    fn suspend_storage(&mut self) -> Result<()> {
        self.backend.suspend_storage()
    }
    fn resume_storage(&mut self) -> Result<Option<State>> {
        self.backend.resume_storage()
    }
}
#[tokio::test]
async fn every_atomic_snapshot_and_log_excludes_ordinary_material() {
    let capture = logging::Capture::default();
    let logs = Arc::clone(&capture.0);
    async move {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("protected.json");
        let rows = Arc::new(Mutex::new(Vec::new()));
        let backend = Snapshots {
            backend: ProtectedFileStorage::open(&path).unwrap(),
            path: path.clone(),
            rows: Arc::clone(&rows),
        };
        let (a, b, w, id) = pair(backend, TransientLimits::default()).await;
        let events = collector(&a);
        let text = "synthetic snapshot content";
        let url = "https://example.invalid/sentinel";
        a.send_message(
            id,
            text,
            Some(vec![
                grammers_tl_types::types::MessageEntityTextUrl {
                    offset: 0,
                    length: 4,
                    url: url.into(),
                }
                .into(),
            ]),
            None,
        )
        .await
        .unwrap();
        pump(&a, &b, &w).await;
        b.send_file(
            id,
            FileSource::bytes(b"synthetic media".to_vec(), "snapshot.bin").unwrap(),
            SendFileOptions::default(),
        )
        .await
        .unwrap();
        pump(&a, &b, &w).await;
        a.settle().await.unwrap();
        let media = events.lock().unwrap()[0]
            .media_reference
            .as_ref()
            .unwrap()
            .validate()
            .unwrap();
        let ordinary = w
            .lock()
            .unwrap()
            .sent
            .iter()
            .find(|effect| !effect.outgoing.service)
            .unwrap()
            .outgoing
            .frame
            .clone();
        let tokens = [
            text.to_owned(),
            url.to_owned(),
            hex::encode(ordinary),
            hex::encode(media.key.key()),
            hex::encode(media.key.iv()),
        ];
        a.stop().await.unwrap();
        assert!(!rows.lock().unwrap().is_empty());
        for row in rows.lock().unwrap().iter() {
            let raw = std::str::from_utf8(row).unwrap();
            assert!(!tokens.iter().any(|token| raw.contains(token)));
            let state: State = serde_json::from_slice(row).unwrap();
            for chat in state.chats.values() {
                assert!(
                    chat.outbox.is_empty()
                        && chat.gaps.is_empty()
                        && chat.deliveries.is_empty()
                        && chat.service_deliveries.is_empty()
                );
            }
        }
        let logs = logs.lock().unwrap().join("\n");
        assert!(!tokens.iter().any(|token| logs.contains(token)));
        let backup = dir.path().join("backup.json");
        std::fs::copy(&path, &backup).unwrap();
        let raw = std::fs::read_to_string(backup).unwrap();
        assert!(!tokens.iter().any(|token| raw.contains(token)));
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_str()
                .unwrap()
                .starts_with(".secret-chat-store-")
        }));
        b.stop().await.unwrap();
    }
    .with_subscriber(capture)
    .await;
}
