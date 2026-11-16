//! Original-client regression coverage for upload, download and message iterators.
#[path = "review_blocking.rs"]
mod blocking_review;
#[path = "review_downloads.rs"]
mod downloads_review;
#[path = "review_updates.rs"]
mod updates_review;
use kiaro_secret_chat::{
    grammers_client::{Client, media::Downloadable},
    grammers_mtsender::{InvocationError, SenderPool},
    grammers_session::{
        SessionData,
        storages::MemorySession,
        types::{PeerAuth, PeerId, PeerRef},
    },
    grammers_tl_types::{self as tl, Identifiable, Serializable},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::AsyncReadExt;
#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[derive(Default)]
struct Model {
    base: super::adapter::server::Server,
    downloads: VecDeque<Result<Vec<u8>, InvocationError>>,
    offsets: Vec<i64>,
    deletions: usize,
    unpins: usize,
    auth_failure: bool,
    imports: usize,
    exports: usize,
    download_gate: Option<Arc<Gate>>,
    search_pages: usize,
}
fn message(id: i32, peer: i64) -> tl::enums::Message {
    let mut bytes = tl::types::MessageService::CONSTRUCTOR_ID
        .to_le_bytes()
        .to_vec();
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(id.to_le_bytes());
    bytes.extend(tl::types::PeerUser::CONSTRUCTOR_ID.to_le_bytes());
    bytes.extend(peer.to_le_bytes());
    bytes.extend(123i32.to_le_bytes());
    bytes.extend(tl::enums::MessageAction::Empty.to_bytes());
    super::decode(&bytes)
}
fn users(ids: impl Iterator<Item = i64>) -> Vec<tl::enums::User> {
    let mut original = super::self_user(false);
    let tl::enums::User::User(user) = super::decode::<Vec<tl::enums::User>>(&original).remove(0)
    else {
        panic!()
    };
    original.clear();
    ids.map(|id| {
        let mut user = user.clone();
        user.id = id;
        user.is_self = false;
        user.access_hash = Some(7);
        user.into()
    })
    .collect()
}
fn boxed<T: Serializable + Identifiable>(value: &T) -> Vec<u8> {
    let mut bytes = T::CONSTRUCTOR_ID.to_le_bytes().to_vec();
    value.serialize(&mut |part| bytes.extend_from_slice(part));
    bytes
}
impl Model {
    fn invoke(&mut self, dc: i32, body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let id = u32::from_le_bytes(body[..4].try_into().unwrap());
        Ok(match id {
            id if id == tl::functions::upload::GetFile::CONSTRUCTOR_ID => {
                let request = super::decode::<tl::functions::upload::GetFile>(&body[4..]);
                self.offsets.push(request.offset);
                if self.auth_failure {
                    return Err(super::adapter::server::rpc("AUTH_KEY_UNREGISTERED"));
                }
                if let Some(reply) = self.downloads.pop_front() {
                    return reply;
                }
                tl::enums::upload::File::File(tl::types::upload::File {
                    r#type: tl::enums::storage::FileType::FileUnknown,
                    mtime: 0,
                    bytes: vec![9],
                })
                .to_bytes()
            }
            id if id == tl::functions::auth::ExportAuthorization::CONSTRUCTOR_ID => {
                self.exports += 1;
                boxed(&tl::types::auth::ExportedAuthorization {
                    id: 123,
                    bytes: vec![7],
                })
            }
            id if id == tl::functions::auth::ImportAuthorization::CONSTRUCTOR_ID => {
                self.imports += 1;
                boxed(&tl::types::auth::Authorization {
                    setup_password_required: false,
                    otherwise_relogin_days: None,
                    tmp_sessions: None,
                    future_auth_token: None,
                    user: users([123].into_iter()).remove(0),
                })
            }
            id if id == tl::functions::messages::DeleteHistory::CONSTRUCTOR_ID => {
                self.deletions += 1;
                boxed(&tl::types::messages::AffectedHistory {
                    pts: 1,
                    pts_count: 1,
                    offset: i32::from(self.deletions == 1),
                })
            }
            id if id == tl::functions::messages::UnpinAllMessages::CONSTRUCTOR_ID => {
                self.unpins += 1;
                boxed(&tl::types::messages::AffectedHistory {
                    pts: 1,
                    pts_count: 1,
                    offset: i32::from(self.unpins == 1),
                })
            }
            id if id == tl::functions::messages::ForwardMessages::CONSTRUCTOR_ID => {
                let request = super::decode::<tl::functions::messages::ForwardMessages>(&body[4..]);
                let mapped = *request.random_id.last().unwrap();
                let mut updates = vec![
                    tl::types::UpdateNewMessage {
                        message: message(17, 456),
                        pts: 1,
                        pts_count: 1,
                    }
                    .into(),
                ];
                if request.random_id.len() > 1 {
                    updates.push(
                        tl::types::UpdateMessageId {
                            id: 17,
                            random_id: mapped,
                        }
                        .into(),
                    );
                }
                tl::enums::Updates::Updates(tl::types::Updates {
                    updates,
                    users: users([456].into_iter()),
                    chats: vec![],
                    date: 123,
                    seq: 1,
                })
                .to_bytes()
            }
            id if id == tl::functions::messages::SearchGlobal::CONSTRUCTOR_ID => {
                self.search_pages += 1;
                let peers = if self.search_pages == 1 {
                    (1000..1100).collect::<Vec<_>>()
                } else if self.search_pages == 2 {
                    vec![999]
                } else {
                    vec![]
                };
                tl::enums::messages::Messages::Slice(tl::types::messages::MessagesSlice {
                    inexact: false,
                    count: 101,
                    next_rate: Some(self.search_pages as i32),
                    offset_id_offset: None,
                    search_flood: None,
                    messages: peers.iter().map(|peer| message(2, *peer)).collect(),
                    topics: vec![],
                    chats: vec![],
                    users: users(peers.into_iter()),
                })
                .to_bytes()
            }
            _ => return self.base.invoke(dc, body),
        })
    }
}
struct File {
    dc: i32,
    size: usize,
}
impl Downloadable for File {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputEncryptedFileLocation {
                id: 7,
                access_hash: 8,
            }
            .into(),
        )
    }
    fn dc_id(&self) -> Option<i32> {
        Some(self.dc)
    }
    fn size(&self) -> Option<usize> {
        Some(self.size)
    }
}
pub async fn run() {
    updates_review::run().await;
    downloads_review::bounded_chunk_progress();
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    for dc in [2, 4] {
        data.dc_options.get_mut(&dc).unwrap().auth_key = Some([7; 256]);
    }
    let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
    let client = Client::new(pool.handle.clone());
    let handle = pool.handle.clone();
    let model = Arc::new(Mutex::new(Model::default()));
    let remote = model.clone();
    let runner = tokio::spawn(pool.runner.run_offline_fixture(move |dc, body| {
        let remote = remote.clone();
        async move {
            let gate = if u32::from_le_bytes(body[..4].try_into().unwrap())
                == tl::functions::upload::GetFile::CONSTRUCTOR_ID
            {
                remote.lock().unwrap().download_gate.take()
            } else {
                None
            };
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            remote.lock().unwrap().invoke(dc, body)
        }
    }));
    let mut source = b"AB".as_slice();
    let uploaded = client
        .upload_stream(&mut source, 1, "prefix.bin".into())
        .await
        .unwrap();
    assert_eq!(
        source, b"B",
        "upload must leave the undeclared suffix unread"
    );
    let tl::enums::InputFile::File(raw) = uploaded.raw else {
        panic!()
    };
    assert_eq!(model.lock().unwrap().base.files[&raw.id], b"A");
    let mut short = b"A".as_slice();
    let error = client
        .upload_stream(&mut short, 2, "short.bin".into())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    let peer = PeerRef {
        id: PeerId::user(456).unwrap(),
        auth: PeerAuth::from_hash(7),
    };
    client.delete_dialog(peer).await.unwrap();
    assert_eq!(model.lock().unwrap().deletions, 2);
    client.unpin_all_messages(peer).await.unwrap();
    assert_eq!(model.lock().unwrap().unpins, 2);
    let forwarded = client.forward_messages(peer, &[1, 2], peer).await.unwrap();
    assert!(forwarded[0].is_none());
    assert_eq!(forwarded[1].as_ref().unwrap().id(), 17);
    assert_eq!(
        client.forward_messages(peer, &[1], peer).await.unwrap()[0]
            .as_ref()
            .unwrap()
            .id(),
        17,
        "the unambiguous single-message fallback remains supported"
    );
    let mut search = client.search_all_messages();
    let mut count = 0;
    while search.next().await.unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 101);
    assert_eq!(model.lock().unwrap().search_pages, 3);
    model
        .lock()
        .unwrap()
        .downloads
        .push_back(Err(super::adapter::server::rpc("TEMPORARY_FAILURE")));
    let mut download = client.iter_download(&File { dc: 2, size: 1 });
    assert!(download.next().await.is_err());
    assert_eq!(download.next().await.unwrap(), Some(vec![9]));
    assert_eq!(model.lock().unwrap().offsets, vec![0, 0]);
    let gate = Arc::new(Gate::default());
    model.lock().unwrap().download_gate = Some(Arc::clone(&gate));
    let before_cancel = model.lock().unwrap().offsets.len();
    let mut interrupted = client.iter_download(&File { dc: 2, size: 1 });
    let mut pending = Box::pin(interrupted.next());
    tokio::select! {_=gate.entered.notified()=>{},result=&mut pending=>panic!("download gate bypassed: {}",result.is_ok())}
    drop(pending);
    gate.release.notify_one();
    assert_eq!(interrupted.next().await.unwrap(), Some(vec![9]));
    assert_eq!(
        &model.lock().unwrap().offsets[before_cancel..],
        &[0, 0],
        "a canceled chunk must be retried at its original offset"
    );
    model.lock().unwrap().auth_failure = true;
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        client.iter_download(&File { dc: 2, size: 1 }).next(),
    )
    .await;
    assert!(
        matches!(result, Ok(Err(InvocationError::Rpc(_)))),
        "home-DC authorization cannot be repaired by importing itself"
    );
    for _ in 0..2 {
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            client.iter_download(&File { dc: 4, size: 1 }).next(),
        )
        .await;
        assert!(
            matches!(result, Ok(Err(InvocationError::Rpc(_)))),
            "a cached import must not cause an infinite authorization loop"
        );
    }
    assert_eq!(model.lock().unwrap().exports, 1);
    assert_eq!(model.lock().unwrap().imports, 1);
    model.lock().unwrap().auth_failure = false;
    let mut suffix = Vec::new();
    source.read_to_end(&mut suffix).await.unwrap();
    assert_eq!(suffix, b"B");
    blocking_review::run(&client, &model).await;
    handle.quit();
    runner.await.unwrap();
    drop(pool.updates);
}
