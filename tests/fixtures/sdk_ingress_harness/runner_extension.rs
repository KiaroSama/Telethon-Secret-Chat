// Compiled ONLY in an isolated test copy of the pinned SDK. The network side
// is replaced; original Client/handles/update receiver/ingress code is used.
#[cfg(feature = "kiaro-offline-test-fixture")]
impl SenderPoolRunner {
    pub fn offline_fixture_wrap_hook(
        &self,
        wrap: impl FnOnce(
            std::sync::Arc<dyn crate::DurableIngressHook>,
        ) -> std::sync::Arc<dyn crate::DurableIngressHook>,
    ) {
        let mut boundary = self.boundary.lock().unwrap();
        assert!(!boundary.started && !boundary.dispatched && !boundary.receiver);
        let original = boundary.hook.take().unwrap();
        boundary.hook = Some(wrap(original));
    }
    pub async fn offline_fixture_store_key(
        &self,
        dc_id: i32,
        key: [u8; 256],
    ) -> Result<(), InvocationError> {
        self.store_authorization_key(dc_id, key).await
    }
    pub async fn offline_fixture_initialization(
        &self,
        frames: Vec<Vec<UpdatesLike>>,
        reply_at: usize,
        staged: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    ) -> (Result<Vec<u8>, InvocationError>, usize) {
        struct Driver {
            frames: std::collections::VecDeque<Vec<UpdatesLike>>,
            response: Option<oneshot::Sender<Result<Vec<u8>, InvocationError>>>,
            reply_at: usize,
            steps: usize,
            required: usize,
            staged: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
        }
        impl InitializationStep for Driver {
            fn initialization_step(
                &mut self,
            ) -> grammers_session::BoxFuture<'_, Result<Vec<UpdatesLike>, ReadError>> {
                Box::pin(async move {
                    if let Some(staged) = &self.staged {
                        assert_eq!(
                            staged.load(std::sync::atomic::Ordering::SeqCst),
                            self.required,
                            "next initialization step could ACK before durable staging"
                        );
                    }
                    self.steps += 1;
                    let frame = self
                        .frames
                        .pop_front()
                        .expect("unexpected initialization network step");
                    self.required += frame.len();
                    if self.steps == self.reply_at {
                        self.response.take().unwrap().send(Ok(vec![41])).unwrap();
                    }
                    Ok(frame)
                })
            }
        }
        assert!((1..=frames.len()).contains(&reply_at));
        let (send, response) = oneshot::channel();
        let mut driver = Driver {
            frames: frames.into(),
            response: Some(send),
            reply_at,
            steps: 0,
            required: 0,
            staged,
        };
        let durable = self.boundary.lock().unwrap().worker();
        let result =
            receive_initialization(&mut driver, response, 2, durable, &self.updates_tx).await;
        (result, driver.steps)
    }
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
