//! Cancellation and persistence probes through the original public UpdateStream.
use kiaro_secret_chat::{
    grammers_client::Client,
    grammers_mtsender::{SenderPool, UpdatesConfiguration},
    grammers_session::{
        BoxFuture, Session, SessionData,
        storages::MemorySession,
        types::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState},
        updates::UpdatesLike,
    },
    grammers_tl_types::{self as tl, Serializable},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;
struct GateSession {
    inner: MemorySession,
    gate: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl Session for GateSession {
    type Error = <MemorySession as Session>::Error;
    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        self.inner.home_dc_id()
    }
    fn set_home_dc_id(&self, id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_home_dc_id(id)
    }
    fn dc_option(&self, id: i32) -> Result<Option<DcOption>, Self::Error> {
        self.inner.dc_option(id)
    }
    fn set_dc_option(&self, value: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_dc_option(value)
    }
    fn peer(&self, id: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        self.inner.peer(id)
    }
    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            if self.gate.swap(false, Ordering::AcqRel) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.cache_peer(peer).await
        })
    }
    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        self.inner.updates_state()
    }
    fn set_update_state(&self, state: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_update_state(state)
    }
}
pub async fn run() {
    let mut data = SessionData {
        home_dc: 2,
        updates_state: UpdatesState {
            pts: 10,
            qts: 40,
            date: 123,
            seq: 1,
            channels: vec![],
        },
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some([7; 256]);
    let session = Arc::new(GateSession {
        inner: MemorySession::from(data),
        gate: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let pool = SenderPool::new(session.clone(), 123);
    let client = Client::new(pool.handle.clone());
    let handle = pool.handle.clone();
    let live = pool.runner.offline_fixture_updates();
    let runner = tokio::spawn(pool.runner.run_offline_fixture(|_, _| async {
        Ok(
            tl::enums::updates::Difference::Empty(tl::types::updates::DifferenceEmpty {
                date: 123,
                seq: 1,
            })
            .to_bytes(),
        )
    }));
    let stream = client
        .stream_updates(pool.updates, UpdatesConfiguration { catch_up: true })
        .await
        .unwrap();
    // This independently owned stream scenario must not compound the parent
    // suite's generator poll frames on Windows' default 1 MiB main stack.
    // JoinSet aborts the child if the outer suite is canceled or times out.
    let mut checks = tokio::task::JoinSet::new();
    checks.spawn(verify_stream(stream, session, live));
    checks.join_next().await.unwrap().unwrap();
    handle.quit();
    runner.await.unwrap();
}

async fn verify_stream(
    mut stream: kiaro_secret_chat::grammers_client::client::UpdateStream,
    session: Arc<GateSession>,
    live: tokio::sync::mpsc::Sender<UpdatesLike>,
) {
    let mut users: Vec<tl::enums::User> = super::super::decode(&super::super::self_user(false));
    let tl::enums::User::User(user) = &mut users[0] else {
        panic!()
    };
    user.is_self = false;
    user.id = 456;
    user.access_hash = Some(77);
    let updates = (41..=42)
        .map(|qts| {
            tl::types::UpdateNewEncryptedMessage {
                message: tl::types::EncryptedMessageService {
                    random_id: i64::from(qts),
                    chat_id: 7,
                    date: 123,
                    bytes: vec![1],
                }
                .into(),
                qts,
            }
            .into()
        })
        .collect();
    session.gate.store(true, Ordering::Release);
    live.send(UpdatesLike::Updates(
        tl::types::Updates {
            updates,
            users,
            chats: vec![],
            date: 124,
            seq: 2,
        }
        .into(),
    ))
    .await
    .unwrap();
    let mut next = Box::pin(stream.next_raw());
    tokio::select! {value=&mut next=>panic!("peer cache gate was bypassed: {}",value.is_ok()),_=session.entered.notified()=>{}}
    drop(next);
    assert!(
        stream.sync_update_state().await.is_err(),
        "a received batch awaiting peer cache cannot be persisted as consumed"
    );
    assert_eq!(session.updates_state().await.unwrap().qts, 40);
    session.release.notify_one();
    let (first, _, _) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        Box::pin(stream.next_raw()),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(first,tl::enums::Update::NewEncryptedMessage(update)if update.qts==41));
    assert!(
        stream.sync_update_state().await.is_err(),
        "a caller has not observed the second buffered update"
    );
    assert_eq!(session.updates_state().await.unwrap().qts, 40);
    let (second, _, _) = Box::pin(stream.next_raw()).await.unwrap();
    assert!(matches!(second,tl::enums::Update::NewEncryptedMessage(update)if update.qts==42));
    stream.sync_update_state().await.unwrap();
    assert_eq!(session.updates_state().await.unwrap().qts, 42);
}
