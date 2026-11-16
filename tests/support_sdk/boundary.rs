#![allow(clippy::unwrap_used)]
use grammers_mtsender::{
    DurableIngressHook, IngressReceiptKind, IngressRefused, SenderPool, UpdatesConfiguration,
    UpdatesReceiver,
};
use grammers_session::{
    storages::MemorySession,
    types::UpdatesState,
    updates::{MessageBoxes, UpdateAndPeers, UpdatesLike},
};
use grammers_tl_types::{self as tl, Serializable};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
struct Hook {
    checkpoint: Mutex<Vec<u8>>,
    receipts: Mutex<Vec<Vec<u8>>>,
    fail: bool,
    pulse: Option<Arc<AtomicBool>>,
    saw_pulse: Arc<AtomicBool>,
}
impl Hook {
    fn new(fail: bool) -> Self {
        Self {
            checkpoint: Mutex::new(
                MessageBoxes::load(UpdatesState {
                    pts: 10,
                    qts: 40,
                    date: 123,
                    seq: 1,
                    channels: vec![],
                })
                .durable_checkpoint()
                .unwrap(),
            ),
            receipts: Mutex::new(vec![]),
            fail,
            pulse: None,
            saw_pulse: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl DurableIngressHook for Hook {
    fn stage_live(&self, _dc_id: i32, _input: &UpdatesLike) -> Result<(), IngressRefused> {
        Ok(())
    }
    fn pending_live(&self) -> Result<Vec<UpdatesLike>, IngressRefused> {
        Ok(vec![])
    }
    fn receipt_notification(&self) -> Arc<tokio::sync::Notify> {
        Arc::new(tokio::sync::Notify::new())
    }
    fn restore(&self) -> Result<Vec<u8>, IngressRefused> {
        Ok(self.checkpoint.lock().unwrap().clone())
    }
    fn replay(&self) -> Result<Vec<(u64, UpdateAndPeers)>, IngressRefused> {
        Ok(vec![])
    }
    fn commit(
        &self,
        _before: &[u8],
        after: &[u8],
        kind: IngressReceiptKind,
        envelope: &[u8],
        _output: &UpdateAndPeers,
    ) -> Result<Option<u64>, IngressRefused> {
        if self.fail {
            return Err(IngressRefused);
        }
        if kind != IngressReceiptKind::Machine {
            self.receipts.lock().unwrap().push(envelope.to_vec());
        }
        *self.checkpoint.lock().unwrap() = after.to_vec();
        Ok(None)
    }
    fn guard_request(&self, _dc: i32, _body: &[u8]) -> Result<u64, IngressRefused> {
        if let Some(pulse) = &self.pulse {
            for _ in 0..100 {
                if pulse.load(Ordering::Acquire) {
                    self.saw_pulse.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Err(IngressRefused)
    }
    fn guard_result(
        &self,
        _permit: u64,
        _dc: i32,
        _request: &[u8],
        _response: &[u8],
    ) -> Result<(), IngressRefused> {
        Ok(())
    }
    fn cancel_request(&self, _permit: u64) {}
}
pub async fn original_handle_clone_refuses_before_request_is_queued() {
    // This future's first poll enqueues before the runner starts. Attachment
    // must refuse even when an original handle already queued that invocation.
    let queued_pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    let mut queued = Box::pin(queued_pool.handle.raw_invoke_in_dc(
        2,
        tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes(),
    ));
    std::future::poll_fn(|context| {
        assert!(queued.as_mut().poll(context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert!(
        queued_pool
            .handle
            .install_durable_ingress(Arc::new(Hook::new(false)))
            .is_err()
    );
    drop(queued);

    let short_pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    for size in 0..4 {
        let refused = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            short_pool.handle.raw_invoke_in_dc(2, vec![0; size]),
        )
        .await
        .unwrap();
        assert!(refused.is_err());
    }

    let pulse = Arc::new(AtomicBool::new(false));
    let mut blocking = Hook::new(false);
    blocking.pulse = Some(Arc::clone(&pulse));
    let saw_pulse = Arc::clone(&blocking.saw_pulse);
    let blocking_pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    blocking_pool
        .handle
        .install_durable_ingress(Arc::new(blocking))
        .unwrap();
    let heartbeat = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        pulse.store(true, Ordering::Release);
    });
    assert!(
        blocking_pool
            .handle
            .raw_invoke_in_dc(
                2,
                tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes(),
            )
            .await
            .is_err()
    );
    assert!(
        saw_pulse.load(Ordering::Acquire),
        "synchronous custody work blocked the current-thread executor"
    );
    heartbeat.await.unwrap();
    let pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    let original = pool.handle.thin.clone();
    let hook = Arc::new(Hook::new(false));
    pool.handle.install_durable_ingress(hook).unwrap();
    let result = original
        .raw_invoke_in_dc(
            2,
            tl::functions::messages::ReceivedQueue { max_qts: 41 }.to_bytes(),
        )
        .await;
    assert!(result.is_err());
    assert!(
        pool.handle
            .install_durable_ingress(Arc::new(Hook::new(false)))
            .is_err()
    );
}
pub async fn receiver_restores_checkpoint_before_initial_state_rpc() {
    let pool = SenderPool::new(Arc::new(MemorySession::default()), 123);
    pool.handle
        .install_durable_ingress(Arc::new(Hook::new(false)))
        .unwrap();
    let _receiver = UpdatesReceiver::create(
        pool.handle.thin.clone(),
        pool.handle.session.clone(),
        pool.updates,
        UpdatesConfiguration::default(),
    )
    .await
    .unwrap();
    // A second update owner on the same original pool would race its checkpoint.
    let (_tx, rx) = tokio::sync::mpsc::channel::<UpdatesLike>(1);
    assert!(
        UpdatesReceiver::create(
            pool.handle.thin.clone(),
            pool.handle.session.clone(),
            rx,
            UpdatesConfiguration::default()
        )
        .await
        .is_err()
    );
}
