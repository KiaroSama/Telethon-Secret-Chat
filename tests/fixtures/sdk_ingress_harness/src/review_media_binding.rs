//! Binding changes must stop native transfer dispatch and prevent publication.
use super::server;
use kiaro_secret_chat::{
    error::Error,
    export::{
        fetch_client::ExportClient, fetch_files::LazyFile, grammers_client::GrammersExportClient,
        model::FileLocation,
    },
    files::{FileKey, encrypt_file},
    grammers_client::Client,
    grammers_mtsender::{SenderPool, SenderPoolHandle},
    grammers_session::{Session, SessionData, storages::MemorySession},
    grammers_tl_types::{self as tl, Identifiable},
    manager::Transport,
    media::{FileSource, MediaReference},
    transport::{EncryptedFileRef, EstablishmentTransport, GrammersTransport},
    types,
};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

const PLAIN_SIZE: usize = 512 * 1024 + 17;
const ORIGINAL: &[u8] = b"the prior destination must survive refusal";

struct Fixture {
    session: Arc<MemorySession>,
    client: Client,
    handle: SenderPoolHandle,
    runner: tokio::task::JoinHandle<()>,
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}
impl Fixture {
    fn new(download: Vec<u8>, pause_at: Option<usize>, failure_at: Option<usize>) -> Self {
        let mut data = SessionData {
            home_dc: 2,
            ..Default::default()
        };
        for dc in [2, 4] {
            data.dc_options.get_mut(&dc).unwrap().auth_key = Some([7; 256]);
        }
        let session = Arc::new(MemorySession::from(data));
        let pool = SenderPool::new(session.clone(), 123);
        let client = Client::new(pool.handle.clone());
        let remote = Arc::new(Mutex::new(server::Server {
            download,
            ..Default::default()
        }));
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let entered = Arc::new(Notify::new());
        let initial = entered.clone();
        let release = Arc::new(Notify::new());
        let resume = release.clone();
        let runner = tokio::spawn(pool.runner.run_offline_fixture(move |dc, body| {
            let remote = remote.clone();
            let count = count.clone();
            let initial = initial.clone();
            let resume = resume.clone();
            async move {
                let id = u32::from_le_bytes(body[..4].try_into().unwrap());
                if id == tl::functions::upload::GetFile::CONSTRUCTOR_ID
                    || id == tl::functions::upload::SaveFilePart::CONSTRUCTOR_ID
                    || id == tl::functions::upload::SaveBigFilePart::CONSTRUCTOR_ID
                {
                    let number = count.fetch_add(1, Ordering::SeqCst) + 1;
                    if pause_at == Some(number) {
                        initial.notify_one();
                        resume.notified().await;
                    }
                    if failure_at == Some(number) {
                        return Err(server::rpc("FILE_REFERENCE_EXPIRED"));
                    }
                }
                remote.lock().unwrap().invoke(dc, body)
            }
        }));
        Self {
            session,
            client,
            handle: pool.handle.thin,
            runner,
            calls,
            entered,
            release,
        }
    }
    async fn resume_after_first(&self, changed: bool) {
        tokio::time::timeout(Duration::from_secs(10), self.entered.notified())
            .await
            .unwrap();
        if changed {
            let mut dc = self.session.dc_option(2).unwrap().unwrap();
            dc.auth_key = Some([9; 256]);
            self.session.set_dc_option(&dc).await.unwrap();
        }
        self.release.notify_one();
    }
    async fn close(self) {
        self.handle.quit();
        tokio::time::timeout(Duration::from_secs(10), self.runner)
            .await
            .unwrap()
            .unwrap();
    }
}

fn media() -> (FileKey, Vec<u8>, Vec<u8>, MediaReference) {
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let plaintext = vec![0x5a; PLAIN_SIZE];
    let mut encrypted = vec![];
    encrypt_file(&mut &plaintext[..], &mut encrypted, PLAIN_SIZE as u64, &key).unwrap();
    let reference = MediaReference::new(
        7,
        &tl::types::EncryptedFile {
            id: 9,
            access_hash: 99,
            size: encrypted.len() as i64,
            dc_id: 4,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: PLAIN_SIZE as i64,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    (key, plaintext, encrypted, reference)
}

#[derive(Clone, Copy, Debug)]
enum Native {
    Disk,
    Ram,
    Upload,
}
async fn native(kind: Native, changed: bool) {
    let (key, plaintext, ciphertext, reference) = media();
    let fixture = Fixture::new(ciphertext, Some(1), None);
    let transport = GrammersTransport::new(fixture.client.clone());
    transport
        .verified_binding("native-media-binding")
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("destination.bin");
    std::fs::write(&path, ORIGINAL).unwrap();
    let destination = path.clone();
    let expected = plaintext.clone();
    let operation = tokio::spawn(async move {
        match kind {
            Native::Disk => Transport::download(&transport, &reference, &destination)
                .await
                .map(|_| ()),
            Native::Ram => Transport::download_bytes(&transport, &reference, PLAIN_SIZE)
                .await
                .map(|bytes| {
                    assert_eq!(bytes, expected);
                }),
            Native::Upload => {
                let mut source = FileSource::bytes(expected, "synthetic.bin")
                    .unwrap()
                    .open()
                    .unwrap();
                Transport::upload(&transport, &mut source, &key)
                    .await
                    .map(|_| ())
            }
        }
    });
    fixture.resume_after_first(changed).await;
    let result = tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .unwrap()
        .unwrap();
    let calls = fixture.calls.load(Ordering::SeqCst);
    if changed {
        assert_eq!(result, Err(Error::Authorization));
        assert_eq!(calls, 1, "native {kind:?} dispatched after binding changed");
        assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL);
    } else {
        result.unwrap();
        assert_eq!(calls, 2);
        if matches!(kind, Native::Disk) {
            assert_eq!(std::fs::read(&path).unwrap(), plaintext);
        }
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    fixture.close().await;
}

async fn core_download(changed: bool) {
    let (key, _, ciphertext, _) = media();
    let expected = ciphertext.clone();
    let fixture = Fixture::new(ciphertext, Some(1), None);
    let transport = GrammersTransport::connect(fixture.client.clone(), "core-control")
        .await
        .unwrap();
    let reference = EncryptedFileRef {
        id: 9,
        access_hash: 99,
        size: expected.len() as i64,
        dc_id: 4,
        key_fingerprint: key.fingerprint(),
    };
    let operation = tokio::spawn(async move {
        EstablishmentTransport::download_encrypted_bytes(&transport, &reference).await
    });
    fixture.resume_after_first(changed).await;
    let result = tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .unwrap()
        .unwrap();
    let calls = fixture.calls.load(Ordering::SeqCst);
    if changed {
        assert!(result.is_err());
        assert_eq!(calls, 1);
    } else {
        assert_eq!(result.unwrap(), expected);
        assert_eq!(calls, 2);
    }
    fixture.close().await;
}

async fn export_length(actual: usize, declared: u64, failure_at: Option<usize>) {
    let fixture = Fixture::new(vec![0x41; actual], None, failure_at);
    let export = GrammersExportClient::new(fixture.client.clone());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("download.bin");
    std::fs::write(&path, ORIGINAL).unwrap();
    let mut target = LazyFile::new(&path);
    let location = FileLocation {
        dc_id: 4,
        data: json!({"_":"InputDocumentFileLocation","id":9,"access_hash":99,"file_reference":[],"thumb_size":""}),
    };
    let result = export.download(&location, declared, &mut target).await;
    if failure_at.is_none() && (declared == 0 || actual as u64 == declared) {
        result.unwrap();
        target.finish().unwrap();
        target.finish().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), vec![0x41; actual]);
    } else {
        assert!(
            result.is_err(),
            "export accepted {actual} bytes for declared {declared}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL);
    }
    target.close();
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    fixture.close().await;
}

async fn cancelled_export() {
    let size = 128 * 1024 + 17;
    let fixture = Fixture::new(vec![0x41; size], Some(2), None);
    let export = GrammersExportClient::new(fixture.client.clone());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("download.bin");
    std::fs::write(&path, ORIGINAL).unwrap();
    let mut target = LazyFile::new(&path);
    let location = FileLocation {
        dc_id: 4,
        data: json!({"_":"InputDocumentFileLocation","id":9,"access_hash":99,"file_reference":[],"thumb_size":""}),
    };
    let mut download = export.download(&location, size as u64, &mut target);
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::select! {
            _ = &mut download => panic!("download completed before its paused second chunk"),
            _ = fixture.entered.notified() => {},
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), ORIGINAL);
    drop(download);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    fixture.release.notify_one();
    export
        .download(&location, size as u64, &mut target)
        .await
        .unwrap();
    target.finish().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), vec![0x41; size]);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 4);
    fixture.close().await;
}

pub async fn run() {
    for changed in [false, true] {
        core_download(changed).await;
        for kind in [Native::Disk, Native::Ram, Native::Upload] {
            native(kind, changed).await;
        }
    }
    for (actual, declared) in [
        (65, 65),
        (16, 65),
        (96, 65),
        (128 * 1024 + 17, 128 * 1024 + 17),
        (128 * 1024 + 16, 128 * 1024 + 17),
        (0, 0),
        (128 * 1024 + 17, 0),
    ] {
        export_length(actual, declared, None).await;
    }
    export_length(128 * 1024 + 17, 128 * 1024 + 17, Some(2)).await;
    cancelled_export().await;
}
