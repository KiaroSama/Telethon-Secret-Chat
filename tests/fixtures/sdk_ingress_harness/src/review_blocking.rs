//! A canceled native upload keeps its blocking source owned until manager stop joins it.
use kiaro_secret_chat::{
    chat::{Chat, ChatState, DhRecord},
    crypto::SharedKey,
    files::FileKey,
    grammers_client::Client,
    manager::{Options, SecretChatManager, Transport},
    media::FileSource,
    storage::{MemoryStorage, State, Storage},
    transport::GrammersTransport,
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{Arc, Mutex, mpsc},
};
struct GatedSource {
    inner: Cursor<Vec<u8>>,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: mpsc::Receiver<()>,
    drop_entered: Option<tokio::sync::oneshot::Sender<()>>,
    drop_release: mpsc::Receiver<()>,
}
impl Read for GatedSource {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            self.release.recv().unwrap();
        }
        self.inner.read(buffer)
    }
}
impl Drop for GatedSource {
    fn drop(&mut self) {
        if let Some(entered) = self.drop_entered.take() {
            let _ = entered.send(());
            let _ = self.drop_release.recv();
        }
    }
}
impl Seek for GatedSource {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(from)
    }
}
pub async fn run(client: &Client, model: &Arc<Mutex<super::Model>>) {
    let transport = GrammersTransport::new(client.clone());
    let binding = transport.verified_binding("session").await.unwrap();
    let mut state = State::empty(binding.digest()).unwrap();
    let mut chat = Chat::new(7, 88, 456, true, ChatState::Ready);
    chat.adopt_key(SharedKey::from_slice(&[7; 256]).unwrap())
        .unwrap();
    chat.dh = Some(DhRecord {
        g: 2,
        prime: super::super::adapter::server::hex_prime(),
    });
    chat.capability_layer = 144;
    chat.rekeyed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    state.chats.insert(7, chat);
    let mut storage = MemoryStorage::default();
    storage.commit(&state).unwrap();
    let manager = SecretChatManager::new(
        transport.clone(),
        storage,
        binding.digest(),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    Box::pin(network_wait_cannot_publish_after_stop(&manager, model)).await;
    let count = model.lock().unwrap().base.files.len();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (drop_entered_tx, drop_entered_rx) = tokio::sync::oneshot::channel();
    let (drop_release_tx, drop_release_rx) = mpsc::channel();
    let source = GatedSource {
        inner: Cursor::new(vec![42; 31]),
        entered: Some(entered_tx),
        release: release_rx,
        drop_entered: Some(drop_entered_tx),
        drop_release: drop_release_rx,
    };
    let mut source = FileSource::stream(Box::new(source), "blocked.bin")
        .unwrap()
        .open()
        .unwrap();
    let native = transport.clone();
    let upload = tokio::spawn(async move {
        native
            .upload(&mut source, &FileKey::generate().unwrap())
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), entered_rx)
        .await
        .unwrap()
        .unwrap();
    upload.abort();
    assert!(upload.await.unwrap_err().is_cancelled());
    let mut stopping = Box::pin(manager.stop());
    std::future::poll_fn(|cx| {
        use std::future::Future;
        assert!(
            stopping.as_mut().poll(cx).is_pending(),
            "stop must join an accepted blocking source"
        );
        std::task::Poll::Ready(())
    })
    .await;
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), drop_entered_rx)
        .await
        .unwrap()
        .unwrap();
    std::future::poll_fn(|cx| {
        use std::future::Future;
        assert!(
            stopping.as_mut().poll(cx).is_pending(),
            "stop must also join disposal of the orphaned source"
        );
        std::task::Poll::Ready(())
    })
    .await;
    drop_release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), stopping)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        model.lock().unwrap().base.files.len(),
        count,
        "a canceled refill must never enqueue an upload part after stop"
    );
    manager.start().await.unwrap();
    manager.stop().await.unwrap();
}

async fn network_wait_cannot_publish_after_stop(
    manager: &SecretChatManager<MemoryStorage, GrammersTransport>,
    model: &Arc<Mutex<super::Model>>,
) {
    use kiaro_secret_chat::{grammers_tl_types as tl, media::MediaReference, types};
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let reference = MediaReference::new(
        7,
        &tl::types::EncryptedFile {
            id: 77,
            access_hash: 88,
            size: 16,
            dc_id: 2,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: 1,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    for in_memory in [false, true] {
        let path = directory
            .path()
            .join(format!("must-not-publish-{in_memory}.bin"));
        let output = path.clone();
        let gate = Arc::new(super::Gate::default());
        model.lock().unwrap().download_gate = Some(gate.clone());
        let before = model.lock().unwrap().offsets.len();
        let downloader = manager.clone();
        let reference = reference.clone();
        let task = tokio::spawn(async move {
            if in_memory {
                downloader
                    .receive_file_with_limit(&reference, 1024)
                    .await
                    .map(|_| ())
            } else {
                downloader.save_file(&reference, &output).await.map(|_| ())
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), gate.entered.notified())
            .await
            .unwrap();
        manager.stop().await.unwrap();
        let finished = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
        assert!(
            matches!(
                finished,
                Ok(Ok(Err(kiaro_secret_chat::error::Error::ManagerStopping)))
            ),
            "stop must cancel and join the owned transfer awaiting its already queued RPC (in_memory={in_memory})"
        );
        assert!(
            !path.exists(),
            "a stopped transfer must not publish a later plaintext file"
        );
        gate.release.notify_one();
        manager.start().await.unwrap();
        assert_eq!(
            model.lock().unwrap().offsets.len(),
            before + 1,
            "the already queued RPC may complete but no subsequent chunk may be requested"
        );
    }
}

pub(super) async fn cancelled_upload_preserves_reader(
    client: &Client,
    model: &Arc<Mutex<super::Model>>,
) {
    let native = GrammersTransport::new(client.clone());
    let mut source = FileSource::bytes(b"caller retains this source".to_vec(), "retained.bin")
        .unwrap()
        .open()
        .unwrap();
    let key = FileKey::generate().unwrap();
    let gate = Arc::new(super::Gate::default());
    model.lock().unwrap().upload_gate = Some(gate.clone());
    let mut upload = Box::pin(native.upload(&mut source, &key));
    tokio::select! {_=gate.entered.notified()=>{},result=&mut upload=>panic!("upload RPC gate bypassed: {}",result.is_ok())}
    drop(upload);
    source.rewind().unwrap();
    let mut recovered = Vec::new();
    source.reader().read_to_end(&mut recovered).unwrap();
    gate.release.notify_one();
    native.verified_binding("session").await.unwrap();
    assert_eq!(
        recovered, b"caller retains this source",
        "canceling a borrowed upload must preserve the original reader; explicit rewind restores already consumed bytes"
    );
}
