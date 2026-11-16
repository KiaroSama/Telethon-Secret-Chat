//! Real initialization staging and serialized encrypted-custody callbacks.
use kiaro_secret_chat::{
    binding::AuthorizationBinding,
    custody::IngressLimits,
    grammers_client::Client,
    grammers_mtsender::{
        DurableIngressHook, IngressReceiptKind, IngressRefused, SenderPool, UpdatesConfiguration,
    },
    grammers_session::{
        Session, SessionData,
        storages::MemorySession,
        updates::{MessageBoxes, UpdateAndPeers, UpdatesLike, durable::encode_live},
    },
    grammers_tl_types::{self as tl, Serializable},
    ingress::{DurableIngress, EncryptedIngressStorage},
};
use sha1::{Digest, Sha1};
use std::future::Future;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
struct Gate {
    entered: tokio::sync::Notify,
    release: Mutex<mpsc::Receiver<()>>,
}
fn gate() -> (Arc<Gate>, Release) {
    let (send, receive) = mpsc::channel();
    (
        Arc::new(Gate {
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(receive),
        }),
        Release(Some(send)),
    )
}
impl Gate {
    fn block(&self) {
        self.entered.notify_one();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    }
}
struct Recorder {
    bodies: Mutex<Vec<Vec<u8>>>,
    staged: Arc<AtomicUsize>,
    fail: Option<usize>,
    block: Option<Arc<Gate>>,
    requests: AtomicUsize,
}
impl Recorder {
    fn new(fail: Option<usize>, block: Option<Arc<Gate>>) -> Arc<Self> {
        Arc::new(Self {
            bodies: Mutex::new(vec![]),
            staged: Arc::new(AtomicUsize::new(0)),
            fail,
            block,
            requests: AtomicUsize::new(0),
        })
    }
}
impl DurableIngressHook for Recorder {
    fn prepare_attachment(&self) -> Result<(), IngressRefused> {
        Ok(())
    }
    fn stage_live(&self, dc: i32, input: &UpdatesLike) -> Result<(), IngressRefused> {
        assert_eq!(dc, 2);
        if let Some(gate) = &self.block {
            gate.block();
        }
        let index = self.staged.load(Ordering::SeqCst);
        if self.fail == Some(index) {
            return Err(IngressRefused);
        }
        self.bodies
            .lock()
            .unwrap()
            .push(encode_live(input).unwrap());
        self.staged.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn pending_live(&self) -> Result<Vec<UpdatesLike>, IngressRefused> {
        Ok(vec![])
    }
    fn receipt_notification(&self) -> Arc<tokio::sync::Notify> {
        Arc::new(tokio::sync::Notify::new())
    }
    fn restore(&self) -> Result<Vec<u8>, IngressRefused> {
        Err(IngressRefused)
    }
    fn replay(&self) -> Result<Vec<(u64, UpdateAndPeers)>, IngressRefused> {
        Ok(vec![])
    }
    fn commit(
        &self,
        _: &[u8],
        _: &[u8],
        _: IngressReceiptKind,
        _: &[u8],
        _: &UpdateAndPeers,
    ) -> Result<Option<u64>, IngressRefused> {
        Err(IngressRefused)
    }
    fn guard_request(&self, _: i32, _: &[u8]) -> Result<u64, IngressRefused> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        Err(IngressRefused)
    }
    fn guard_result(&self, _: u64, _: i32, _: &[u8], _: &[u8]) -> Result<(), IngressRefused> {
        Err(IngressRefused)
    }
    fn cancel_request(&self, _: u64) {}
}
fn input(chat: i32) -> UpdatesLike {
    let raw: tl::enums::Updates = tl::types::UpdateShort {
        update: tl::types::UpdateEncryptedChatTyping { chat_id: chat }.into(),
        date: 123,
    }
    .into();
    UpdatesLike::captured(raw.to_bytes(), None)
}
async fn initialization() {
    for later in [false, true] {
        let pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
        let recorder = Recorder::new(None, None);
        pool.handle
            .install_durable_ingress(recorder.clone())
            .unwrap();
        let frames = if later {
            vec![vec![input(1)], vec![input(2)]]
        } else {
            vec![vec![input(1), input(2)]]
        };
        let (result, steps) = pool
            .runner
            .offline_fixture_initialization(
                frames,
                if later { 2 } else { 1 },
                Some(recorder.staged.clone()),
            )
            .await;
        assert_eq!(result.unwrap(), [41]);
        assert_eq!(steps, if later { 2 } else { 1 });
        assert_eq!(
            *recorder.bodies.lock().unwrap(),
            [
                encode_live(&input(1)).unwrap(),
                encode_live(&input(2)).unwrap()
            ]
        );
    }
    for fail in [0, 1] {
        let pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
        let recorder = Recorder::new(Some(fail), None);
        pool.handle
            .install_durable_ingress(recorder.clone())
            .unwrap();
        let (result, steps) = pool
            .runner
            .offline_fixture_initialization(
                vec![vec![input(1), input(2)]],
                1,
                Some(recorder.staged.clone()),
            )
            .await;
        assert!(result.is_err());
        assert_eq!(steps, 1);
        assert_eq!(recorder.staged.load(Ordering::SeqCst), fail);
    }
    // Ordinary SDK initialization also forwards updates from its final read.
    let mut pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    let (result, steps) = pool
        .runner
        .offline_fixture_initialization(vec![vec![input(1), input(2)]], 1, None)
        .await;
    assert_eq!(result.unwrap(), [41]);
    assert_eq!(steps, 1);
    assert_eq!(
        encode_live(&pool.updates.recv().await.unwrap()).unwrap(),
        encode_live(&input(1)).unwrap()
    );
    assert_eq!(
        encode_live(&pool.updates.recv().await.unwrap()).unwrap(),
        encode_live(&input(2)).unwrap()
    );
    // A canceled waiter cannot release the ordering permit while staging still runs.
    let pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    let (gate, release) = gate();
    let recorder = Recorder::new(None, Some(gate.clone()));
    pool.handle
        .install_durable_ingress(recorder.clone())
        .unwrap();
    let mut initializing = Box::pin(pool.runner.offline_fixture_initialization(
        vec![vec![input(1)]],
        1,
        Some(recorder.staged.clone()),
    ));
    tokio::select! {result=&mut initializing=>panic!("staging did not pause: {result:?}"),_=gate.entered.notified()=>{}}
    drop(initializing);
    let request = tl::functions::Ping { ping_id: 7 }.to_bytes();
    let mut queued = Box::pin(pool.handle.raw_invoke_in_dc(2, request));
    std::future::poll_fn(|cx| {
        assert!(queued.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(recorder.requests.load(Ordering::SeqCst), 0);
    drop(release);
    assert!(
        tokio::time::timeout(Duration::from_secs(5), queued)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(recorder.staged.load(Ordering::SeqCst), 1);
    assert_eq!(recorder.requests.load(Ordering::SeqCst), 1);
}
struct Proxy {
    inner: Arc<dyn DurableIngressHook>,
    gate: Arc<Gate>,
    restores: AtomicUsize,
    requests: AtomicUsize,
    commits: Mutex<Vec<IngressReceiptKind>>,
}
impl DurableIngressHook for Proxy {
    fn prepare_attachment(&self) -> Result<(), IngressRefused> {
        self.inner.prepare_attachment()
    }
    fn stage_live(&self, dc: i32, input: &UpdatesLike) -> Result<(), IngressRefused> {
        self.inner.stage_live(dc, input)
    }
    fn pending_live(&self) -> Result<Vec<UpdatesLike>, IngressRefused> {
        self.inner.pending_live()
    }
    fn receipt_notification(&self) -> Arc<tokio::sync::Notify> {
        self.inner.receipt_notification()
    }
    fn restore(&self) -> Result<Vec<u8>, IngressRefused> {
        let bytes = self.inner.restore()?;
        let mut remaining = self.restores.load(Ordering::SeqCst);
        while remaining != 0 {
            match self.restores.compare_exchange_weak(
                remaining,
                remaining - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    if remaining == 1 {
                        self.gate.block();
                    }
                    break;
                }
                Err(current) => remaining = current,
            }
        }
        Ok(bytes)
    }
    fn replay(&self) -> Result<Vec<(u64, UpdateAndPeers)>, IngressRefused> {
        self.inner.replay()
    }
    fn commit(
        &self,
        before: &[u8],
        after: &[u8],
        kind: IngressReceiptKind,
        envelope: &[u8],
        output: &UpdateAndPeers,
    ) -> Result<Option<u64>, IngressRefused> {
        let result = self.inner.commit(before, after, kind, envelope, output)?;
        self.commits.lock().unwrap().push(kind);
        Ok(result)
    }
    fn guard_request(&self, dc: i32, body: &[u8]) -> Result<u64, IngressRefused> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.inner.guard_request(dc, body)
    }
    fn guard_result(
        &self,
        permit: u64,
        dc: i32,
        request: &[u8],
        response: &[u8],
    ) -> Result<(), IngressRefused> {
        self.inner.guard_result(permit, dc, request, response)
    }
    fn cancel_request(&self, permit: u64) {
        self.inner.cancel_request(permit)
    }
}
async fn concurrent_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let key = [7; 256];
    let digest = Sha1::digest(key);
    let auth = i64::from_le_bytes(digest[12..20].try_into().unwrap());
    let binding =
        AuthorizationBinding::from_session_parts(123, auth, 2, "ordered-sdk-fixture".into())
            .unwrap();
    let backend = EncryptedIngressStorage::open(
        directory.path().join("custody"),
        [9; 32],
        binding,
        1024 * 1024,
    )
    .unwrap();
    let ingress = DurableIngress::new(
        backend,
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some(key);
    let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
    let client = Client::new(pool.handle.clone());
    let handle = pool.handle.clone();
    let mut machine = MessageBoxes::new();
    machine.set_state(super::super::state(40));
    ingress
        .prepare(&pool.handle, Some(machine.durable_checkpoint().unwrap()))
        .await
        .unwrap();
    let (gate, release) = gate();
    let proxy = Arc::new(Proxy {
        inner: ingress.clone(),
        gate: gate.clone(),
        restores: AtomicUsize::new(0),
        requests: AtomicUsize::new(0),
        commits: Mutex::new(vec![]),
    });
    let wrapped = proxy.clone();
    pool.runner.offline_fixture_wrap_hook(move |_| wrapped);
    let remote = Arc::new(Mutex::new(super::super::Server {
        owed: Default::default(),
        requests: vec![],
        difference: false,
        bot: false,
    }));
    let model = remote.clone();
    let live = pool.runner.offline_fixture_updates();
    let server = tokio::spawn(pool.runner.run_offline_fixture(move |dc, body| {
        assert_eq!(dc, 2);
        let model = model.clone();
        async move { model.lock().unwrap().invoke(dc, body) }
    }));
    let transport = ingress.transport(client.clone()).await.unwrap();
    let mut receiver = client
        .stream_updates(pool.updates, UpdatesConfiguration::default())
        .await
        .unwrap();
    proxy.restores.store(2, Ordering::SeqCst); // refresh first, atomic transition restore second.
    let receiving = tokio::spawn(async move { receiver.next_raw().await });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    let before = proxy.requests.load(Ordering::SeqCst);
    let request = tl::functions::updates::GetDifference {
        pts: 10,
        pts_limit: None,
        pts_total_limit: None,
        date: 123,
        qts: 40,
        qts_limit: None,
    };
    let mut difference = Box::pin(client.invoke(&request));
    std::future::poll_fn(|cx| {
        assert!(difference.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(
        proxy.requests.load(Ordering::SeqCst),
        before,
        "caller bypassed the in-progress transition's ordering permit"
    );
    assert!(remote.lock().unwrap().requests.is_empty());
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), difference)
        .await
        .unwrap()
        .unwrap();
    live.send(input(7)).await.unwrap();
    let (update, _, _) = tokio::time::timeout(Duration::from_secs(5), receiving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(update,tl::enums::Update::EncryptedChatTyping(typing) if typing.chat_id==7));
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        40
    );
    transport.require_strong_ingress().unwrap();
    client
        .invoke(&tl::functions::Ping { ping_id: 999 })
        .await
        .unwrap();
    assert_eq!(
        proxy.commits.lock().unwrap().first(),
        Some(&IngressReceiptKind::Machine)
    );
    handle.quit();
    server.await.unwrap();
}
async fn refreshed_addresses() {
    let session = Arc::new(MemorySession::default());
    let pool = SenderPool::new(session.clone(), 123);
    let original = session.dc_option(2).unwrap().unwrap();
    let mut fresh = original.clone();
    fresh.ipv4 = "127.0.0.77:7777".parse().unwrap();
    fresh.ipv6 = "[::1]:7777".parse().unwrap();
    session.set_dc_option(&fresh).await.unwrap();
    pool.runner
        .offline_fixture_store_key(2, [9; 256])
        .await
        .unwrap();
    let saved = session.dc_option(2).unwrap().unwrap();
    assert_eq!((saved.ipv4, saved.ipv6), (fresh.ipv4, fresh.ipv6));
    assert_eq!(saved.auth_key, Some([9; 256]));
    assert_ne!(saved.ipv4, original.ipv4);
}
fn runtime_independent_permits() {
    let directory = tempfile::tempdir().unwrap();
    let key = [7; 256];
    let digest = Sha1::digest(key);
    let auth = i64::from_le_bytes(digest[12..20].try_into().unwrap());
    let binding =
        AuthorizationBinding::from_session_parts(123, auth, 2, "short-lived-rpc-runtime".into())
            .unwrap();
    let backend = EncryptedIngressStorage::open(
        directory.path().join("custody"),
        [9; 32],
        binding,
        1024 * 1024,
    )
    .unwrap();
    let ingress = DurableIngress::new(
        backend,
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some(key);
    let mut pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
    let mut machine = MessageBoxes::new();
    machine.set_state(super::super::state(40));
    let runtime = || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    };
    runtime()
        .block_on(ingress.prepare(&pool.handle, Some(machine.durable_checkpoint().unwrap())))
        .unwrap();
    for index in 0..66 {
        let body = tl::functions::Ping { ping_id: index }.to_bytes();
        let success = index == 65;
        let response = if success {
            Ok(tl::types::Pong {
                msg_id: 0,
                ping_id: index,
            }
            .to_bytes())
        } else {
            Err(grammers_mtsender::InvocationError::Rpc(
                tl::types::RpcError {
                    error_code: 400,
                    error_message: "DATA_INVALID".into(),
                }
                .into(),
            ))
        };
        // The final poll returns the RPC error, then the runtime is destroyed
        // before any newly spawned asynchronous cleanup could be polled.
        let short_lived = runtime();
        short_lived.block_on(async {
            let (result, (dc, request)) = tokio::time::timeout(Duration::from_secs(2), async {
                tokio::join!(
                    pool.handle.raw_invoke_in_dc(2, body.clone()),
                    pool.runner.offline_fixture_reply_once(response),
                )
            })
            .await
            .unwrap_or_else(|_| panic!("original request admission leaked at {index}"));
            assert_eq!((dc, request), (2, body));
            if success {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(result, Err(grammers_mtsender::InvocationError::Rpc(_))),
                    "{result:?}"
                );
            }
        });
        drop(short_lived);
    }
}
pub async fn run() {
    tokio::task::spawn_blocking(runtime_independent_permits)
        .await
        .unwrap();
    Box::pin(initialization()).await;
    Box::pin(concurrent_checkpoint()).await;
    Box::pin(refreshed_addresses()).await;
    eprintln!("Native SDK ordered custody, initialization and address-refresh controls passed");
}
