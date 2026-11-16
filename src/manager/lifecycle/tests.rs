#![allow(clippy::unwrap_used)]
use super::*;
use crate::test_support::support;
use proptest::prelude::*;
use support::*;
use tracing::instrument::WithSubscriber;
fn running(manager: &SecretChatManager<TestStorage, Fake>) -> bool {
    manager.inner.running.load(Ordering::Acquire) && !manager.inner.stopping.load(Ordering::Acquire)
}
async fn establish(
    a: &SecretChatManager<TestStorage, Fake>,
    b: &SecretChatManager<TestStorage, Fake>,
    world: &Arc<Mutex<World>>,
) -> Option<i32> {
    let id = a.create(peer(2000)).await.ok()?.id;
    pump(a, b, world).await;
    b.accept(id).await.ok()?;
    pump(a, b, world).await;
    (a.status(id).ok()?.state == ChatState::Ready && b.status(id).ok()?.state == ChatState::Ready)
        .then_some(id)
}
fn invariant(manager: &SecretChatManager<TestStorage, Fake>, store: &TestStorage) {
    if running(manager) {
        let memory = manager.protocol(|p| Ok(p.chat_records())).unwrap();
        let persisted = store.state.lock().unwrap().as_ref().unwrap().chats.clone();
        assert_eq!(memory.len(), persisted.len());
        for chat in memory {
            assert_eq!(
                serde_json::to_value(&chat).unwrap(),
                serde_json::to_value(&persisted[&chat.id]).unwrap()
            );
            if chat.state == ChatState::Closed {
                assert!(
                    chat.key.is_none()
                        && chat.previous_key.is_none()
                        && chat.exchange.is_none()
                        && chat.handshake_secret.is_none()
                        && chat.peer_public.is_none()
                );
                assert!(
                    chat.outbox.is_empty()
                        && chat.gaps.is_empty()
                        && chat.deliveries.is_empty()
                        && chat.service_deliveries.is_empty()
                );
            }
        }
        assert!(manager.protocol(|p| Ok(p.is_running())).unwrap());
    }
    assert_eq!(manager.active_handler_tasks(), 0);
}
proptest! {
 #![proptest_config(ProptestConfig{cases:100,rng_seed:proptest::test_runner::RngSeed::Fixed(0x534543524554),..ProptestConfig::default()})]
 #[test]
 fn stateful_lifecycle_preserves_storage_and_terminal_invariants(trace in proptest::collection::vec((0u8..7,any::<bool>(),0usize..3,proptest::collection::vec(any::<char>(),0..=20),0i32..=3600),25)){
 let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();runtime.block_on(async{
 let(aa,bb)=Fake::pair();let world=Arc::clone(&aa.world);let sa=TestStorage::default();let sb=TestStorage::default();let a=SecretChatManager::new(aa,sa.clone(),"11".repeat(32),Options::default()).unwrap();let b=SecretChatManager::new(bb,sb.clone(),"22".repeat(32),Options::default()).unwrap();a.start().await.unwrap();b.start().await.unwrap();let mut ids=vec![establish(&a,&b,&world).await.unwrap()];
 for (operation,side,index,text,seconds)in trace{let(manager,store)=if side{(&a,&sa)}else{(&b,&sb)};let id=ids[index%ids.len()];match operation{
 0=>{let text:String=text.into_iter().collect();let _=manager.send_message(id,&text,None,None).await;},
 1=>{let _=manager.set_ttl(id,seconds).await;},
 2=>{let _=manager.close(id).await;},
 3=>{if manager.stop().await.is_ok(){assert!(!running(manager));assert!(!manager.protocol(|p|Ok(p.is_running())).unwrap());}},
 4=>{if manager.start().await.is_ok(){assert!(running(manager));}},
 5=>store.failures.store(1,Ordering::Release),
 6=>{if running(&a)&&running(&b)&&ids.len()<3&&let Some(id)=establish(&a,&b,&world).await{ids.push(id);}},
 _=>unreachable!(),}
 pump(&a,&b,&world).await;invariant(&a,&sa);invariant(&b,&sb);
 }
 sa.failures.store(0,Ordering::Release);sb.failures.store(0,Ordering::Release);a.stop().await.unwrap();a.stop().await.unwrap();b.stop().await.unwrap();b.stop().await.unwrap();assert!(!running(&a)&&!running(&b));
 });
 }
}
struct CommitGate {
    entered: Notify,
    released: std::sync::Condvar,
    ready: Mutex<bool>,
}
impl CommitGate {
    fn release(&self) {
        *self.ready.lock().unwrap() = true;
        self.released.notify_all();
    }
}
struct ReleaseCommit(Arc<CommitGate>);
impl Drop for ReleaseCommit {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[derive(Clone)]
struct GateStore {
    store: TestStorage,
    gate: Arc<Mutex<Option<Arc<CommitGate>>>>,
}
impl Storage for GateStore {
    fn load(&self) -> Result<Option<crate::storage::State>> {
        self.store.load()
    }
    fn commit(&mut self, state: &crate::storage::State) -> Result<()> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.notify_one();
            let mut ready = gate.ready.lock().unwrap();
            while !*ready {
                ready = gate.released.wait(ready).unwrap();
            }
            return Err(Error::MediaKind {
                kind: "private lifecycle failure details".into(),
            });
        }
        self.store.commit(state)
    }
}
async fn cancelled_lifecycle(kind: u8) {
    let capture = logging::Capture::default();
    let logs = Arc::clone(&capture.0);
    let _subscriber = tracing::subscriber::set_default(capture);
    let store = TestStorage::default();
    let mut state = crate::storage::State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, seeded::chat());
    *store.state.lock().unwrap() = Some(state);
    let gate = Arc::new(CommitGate {
        entered: Notify::new(),
        released: std::sync::Condvar::new(),
        ready: Mutex::new(false),
    });
    let release = ReleaseCommit(Arc::clone(&gate));
    let slots = Arc::new(Mutex::new(None));
    let (client, _) = Fake::pair();
    let manager = SecretChatManager::new(
        client,
        GateStore {
            store,
            gate: Arc::clone(&slots),
        },
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    *slots.lock().unwrap() = Some(Arc::clone(&gate));
    let first = manager.clone();
    let observer = tokio::spawn(async move { first.stop().await }.with_current_subscriber());
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    if kind == 2 {
        observer.abort();
        assert!(observer.await.unwrap_err().is_cancelled());
    } else {
        let second = manager.clone();
        let waiter = tokio::spawn(async move {
            if kind == 0 {
                second.start().await
            } else {
                second.stop().await
            }
        });
        tokio::task::yield_now().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        release.0.release();
        assert!(observer.await.unwrap().is_err());
    }
    release.0.release();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let complete = manager
                .inner
                .shutdown
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .result
                .lock()
                .unwrap()
                .is_some();
            if complete {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let logs = logs.lock().unwrap().join("\n");
    assert!(logs.contains("shutdown failed"));
    assert!(!logs.contains("private lifecycle failure details"));
    manager.stop().await.unwrap();
}
use crate::test_support::logging;
use crate::test_support::seeded;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_start_waiter_does_not_log_raw_shutdown_failure() {
    cancelled_lifecycle(0).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_stop_waiter_does_not_log_raw_shutdown_failure() {
    cancelled_lifecycle(1).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_only_shutdown_waiter_keeps_observed_redacted_failure() {
    cancelled_lifecycle(2).await;
}
#[tokio::test]
async fn async_callback_registered_while_stopping_is_not_scheduled() {
    let (a, _store, _world, id) = seeded::ready(Options::default()).await;
    let ran = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&ran);
    a.on_async("ChatReady", "late", move |_| {
        let seen = Arc::clone(&seen);
        async move {
            seen.store(true, Ordering::Release);
            Ok(())
        }
    })
    .unwrap();
    a.inner.stopping.store(true, Ordering::Release);
    a.emit(Event::ChatReady {
        chat_id: id,
        peer_user_id: 2000,
        key_fingerprint: 7,
        key_hash: None,
    });
    assert_eq!(a.active_handler_tasks(), 0);
    tokio::task::yield_now().await;
    assert!(!ran.load(Ordering::Acquire));
    a.stop().await.unwrap();
}
struct Dropped(Arc<AtomicBool>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
#[tokio::test]
async fn returned_task_during_shutdown_is_cancelled_before_poll() {
    let (a, _store, _world, id) = seeded::ready(Options::default()).await;
    let ran = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&ran);
    let cleaned = Arc::clone(&dropped);
    a.on_task("ChatReady", "returned", move |_| {
        let seen = Arc::clone(&seen);
        let guard = Dropped(Arc::clone(&cleaned));
        tokio::spawn(async move {
            let _guard = guard;
            seen.store(true, Ordering::Release);
            Ok(())
        })
    })
    .unwrap();
    a.inner.stopping.store(true, Ordering::Release);
    a.emit(Event::ChatReady {
        chat_id: id,
        peer_user_id: 2000,
        key_fingerprint: 7,
        key_hash: None,
    });
    tokio::time::timeout(Duration::from_secs(3), a.settle())
        .await
        .unwrap()
        .unwrap();
    assert!(dropped.load(Ordering::Acquire));
    assert!(!ran.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
    a.stop().await.unwrap();
}
#[tokio::test]
async fn sending_while_stopping_refuses_before_transport() {
    let (a, _store, world, id) = seeded::ready(Options::default()).await;
    a.inner.stopping.store(true, Ordering::Release);
    assert!(matches!(
        a.send_message(id, "late", None, None).await,
        Err(Error::ManagerStopping)
    ));
    assert!(world.lock().unwrap().sent.is_empty());
    a.stop().await.unwrap();
}
#[tokio::test]
async fn handler_tasks_are_owned_and_cancelled_on_stop() {
    let (a, _store, _world, id) = seeded::ready(Options::default()).await;
    let entered = Arc::new(Notify::new());
    let signal = Arc::clone(&entered);
    let cleaned = Arc::new(AtomicBool::new(false));
    let mark = Arc::clone(&cleaned);
    a.on_async_with_cleanup(
        "ChatReady",
        "owned",
        move |_| {
            let signal = Arc::clone(&signal);
            async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            }
        },
        move |_| {
            let mark = Arc::clone(&mark);
            async move {
                mark.store(true, Ordering::Release);
                Ok(())
            }
        },
    )
    .unwrap();
    a.emit(Event::ChatReady {
        chat_id: id,
        peer_user_id: 2000,
        key_fingerprint: 7,
        key_hash: None,
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    assert_eq!(a.active_handler_tasks(), 1);
    a.stop().await.unwrap();
    assert!(cleaned.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
}
#[tokio::test]
async fn cancelling_before_callback_poll_drops_constructed_future() {
    let (a, _store, _world, id) = seeded::ready(Options::default()).await;
    let made = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&made);
    let dropped = Arc::new(AtomicBool::new(false));
    let released = Arc::clone(&dropped);
    a.on_async("ChatReady", "future", move |_| {
        seen.store(true, Ordering::Release);
        let guard = Dropped(Arc::clone(&released));
        async move {
            let _guard = guard;
            std::future::pending::<Result<()>>().await
        }
    })
    .unwrap();
    a.emit(Event::ChatReady {
        chat_id: id,
        peer_user_id: 2000,
        key_fingerprint: 7,
        key_hash: None,
    });
    assert!(made.load(Ordering::Acquire));
    a.stop().await.unwrap();
    assert!(dropped.load(Ordering::Acquire));
    assert_eq!(a.active_handler_tasks(), 0);
}
