// Compiled ONLY in an isolated test copy of the pinned SDK. The network side
// is replaced; original Client/handles/update receiver/ingress code is used.
#[cfg(feature = "kiaro-offline-test-fixture")]
impl SenderPoolRunner {
    pub fn offline_fixture_updates(&self) -> mpsc::Sender<UpdatesLike> {
        self.updates_tx.clone()
    }
    pub async fn run_offline_fixture<F, Fut>(mut self, mut server: F)
    where
        F: FnMut(i32, Vec<u8>) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, InvocationError>>,
    {
        self.boundary.lock().expect("fixture boundary").started = true;
        while let Some(request) = self.request_rx.recv().await {
            match request {
                Request::Invoke { dc_id, body, tx } => {
                    let _ = tx.send(server(dc_id, body).await);
                }
                Request::CheckRetry { error, tx, .. } => {
                    let _ = tx.send(ControlFlow::Break(error));
                }
                Request::Disconnect { .. } => {}
                Request::Quit => break,
            }
        }
    }
}
