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
use std::sync::{Arc, Mutex};
struct Hook {
    checkpoint: Mutex<Vec<u8>>,
    receipts: Mutex<Vec<Vec<u8>>>,
    fail: bool,
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
