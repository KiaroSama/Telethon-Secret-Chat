impl<T: Transport> StrongSecretChatManager<T> {
    /// Detached snapshots of the durable protocol registry.
    pub fn list(&self) -> Result<Vec<ChatSnapshot>> {
        self.inner.manager.list()
    }
    /// Inspect a known conversation without exposing its keys.
    pub fn status(&self, id: i32) -> Result<ChatSnapshot> {
        self.inner.manager.status(id)
    }
    /// Refuse new chat requests while existing conversations continue.
    pub fn set_accepting_requests(&self, enabled: bool) -> Result<()> {
        self.ready()?;
        self.inner.manager.set_accepting_requests(enabled)
    }
    /// Inspect whether local participation is suspended.
    pub fn is_suspended(&self, id: i32) -> Result<bool> {
        self.inner.manager.is_suspended(id)
    }
    /// Pause locally while keeping all journal and consumer obligations.
    pub fn pause_chat(&self, id: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.pause_chat(id)
    }
    /// Inspect continuity and the reason participation is paused.
    pub fn pause_status(&self, id: i32) -> Result<crate::custody::PauseStatus> {
        self.inner.manager.pause_status(id)
    }
    /// Check finite custody without evicting required records.
    pub fn check_custody(&self) -> Result<Vec<i32>> {
        self.inner.manager.check_custody()
    }
    /// Resume intact custody, then revisit committed input from paused peers.
    pub async fn resume_chat(&self, id: i32) -> Result<()> {
        self.ready()?;
        let wake = ReplayWake::new(self)?;
        let result = {
            let operation = self.inner.manager.resume_chat(id);
            operation.await
        };
        wake.finish(result).await
    }
    /// Pending consumer receipts use the original wire sequence and ciphertext identity.
    pub fn pending_deliveries(&self, id: i32) -> Result<Vec<crate::custody::RelayDelivery>> {
        self.inner.manager.pending_relay_deliveries(id)
    }
    /// Acknowledge only after the consumer has durably handled the receipt.
    pub fn acknowledge_delivery(&self, id: i32, wire_sequence: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.acknowledge_delivery(id, wire_sequence)
    }
    /// Strong custody keeps no volatile history cache; use pending_deliveries.
    pub fn read_history(&self, id: i32, limit: usize) -> Result<Vec<MessageReceived>> {
        self.inner.manager.read_history(id, limit)
    }
    /// Await callbacks already owned at entry under the configured deadline.
    pub async fn settle(&self) -> Result<()> {
        self.inner.manager.settle().await
    }
    /// Register an inline Result-returning event callback.
    pub fn on(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> Result<()> + Send + Sync + 'static,
    ) -> Result<()> {
        self.inner.manager.on(event, name, handler)
    }
    /// Register an owned asynchronous callback under the shared task/deadline budget.
    pub fn on_async<F, Fut>(&self, event: &str, name: &'static str, handler: F) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner.manager.on_async(event, name, handler)
    }
    /// Register asynchronous cancellation cleanup for an owned callback.
    pub fn on_async_with_cleanup<F, Fut, C, Cut>(
        &self,
        event: &str,
        name: &'static str,
        handler: F,
        cleanup: C,
    ) -> Result<()>
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner
            .manager
            .on_async_with_cleanup(event, name, handler, cleanup)
    }
    /// Create using the caller client's authorization and retained request identity.
    pub async fn create(&self, user: tl::enums::InputUser) -> Result<ChatSnapshot> {
        self.ready()?;
        let wake = ReplayWake::new(self)?;
        let result = {
            let operation = self.inner.manager.create(user);
            operation.await
        };
        wake.finish(result).await
    }
    /// Accept a durably held incoming request.
    pub async fn accept(&self, id: i32) -> Result<ChatSnapshot> {
        self.ready()?;
        let wake = ReplayWake::new(self)?;
        let result = {
            let operation = self.inner.manager.accept(id);
            operation.await
        };
        wake.finish(result).await
    }
    /// Send content through the durable retained outbox.
    pub async fn send_message(
        &self,
        id: i32,
        text: &str,
        entities: Option<Vec<tl::enums::MessageEntity>>,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.ready()?;
        self.inner
            .manager
            .send_message(id, text, entities, reply_to)
            .await
    }
    /// Send text with a per-message TTL, reply, entities, and silent delivery option.
    pub async fn send_message_with_options(
        &self,
        id: i32,
        text: &str,
        options: SendMessageOptions,
    ) -> Result<i64> {
        self.ready()?;
        self.inner
            .manager
            .send_message_with_options(id, text, options)
            .await
    }
    /// Retry the exact retained ciphertext and random IDs.
    pub async fn retry_pending(&self, id: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.retry_pending(id).await
    }
    /// Request a four-step PFS key exchange.
    pub async fn rekey(&self, id: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.rekey(id).await
    }
    /// Change the message TTL by a sequenced service action.
    pub async fn set_ttl(&self, id: i32, seconds: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.set_ttl(id, seconds).await
    }
    /// Send read receipts for the selected message random IDs.
    pub async fn mark_read(&self, id: i32, random_ids: Vec<i64>) -> Result<()> {
        self.ready()?;
        self.inner.manager.mark_read(id, random_ids).await
    }
    /// Report screenshots for the selected message random IDs.
    pub async fn screenshot(&self, id: i32, random_ids: Vec<i64>) -> Result<()> {
        self.ready()?;
        self.inner.manager.screenshot(id, random_ids).await
    }
    /// Send an activity indicator.
    pub async fn set_typing(
        &self,
        id: i32,
        action: Option<crate::enums::SendMessageAction>,
    ) -> Result<()> {
        self.ready()?;
        self.inner.manager.set_typing(id, action).await
    }
    /// Upload one validated media source through the same client.
    pub async fn send_file(
        &self,
        id: i32,
        source: crate::media::FileSource,
        options: SendFileOptions,
    ) -> Result<i64> {
        self.ready()?;
        self.inner.manager.send_file(id, source, options).await
    }
    /// Forward an encrypted file handle without moving its bytes.
    pub async fn forward_file(
        &self,
        id: i32,
        reference: &crate::media::MediaReference,
        caption: &str,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.ready()?;
        self.inner
            .manager
            .forward_file(id, reference, caption, reply_to)
            .await
    }
    /// Receive validated plaintext in bounded RAM without an archive.
    pub async fn receive_file(&self, reference: &crate::media::MediaReference) -> Result<Vec<u8>> {
        self.ready()?;
        self.inner.manager.receive_file(reference).await
    }
    /// Stop participation without remote discard or custody deletion.
    pub async fn close(&self, id: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.close(id).await
    }
    /// Strong custody does not enable plaintext automatic saving.
    pub async fn start_auto_save_secret_chats(&self, _folder: &std::path::Path) -> Result<()> {
        Err(Error::IngressUnavailable)
    }
    /// Report the active archive destination, which is always absent in this mode.
    pub fn auto_save_secret_chats(&self) -> Option<std::path::PathBuf> {
        None
    }
    /// Register an existing task under the same owned callback budget.
    pub fn on_task(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        self.inner.manager.on_task(event, name, handler)
    }
    /// Attach asynchronous cancellation cleanup to an externally created task.
    pub fn on_task_with_cleanup<C, Cut>(
        &self,
        event: &str,
        name: &'static str,
        handler: impl Fn(Event) -> JoinHandle<Result<()>> + Send + Sync + 'static,
        cleanup: C,
    ) -> Result<()>
    where
        C: Fn(Event) -> Cut + Send + Sync + 'static,
        Cut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        self.inner
            .manager
            .on_task_with_cleanup(event, name, handler, cleanup)
    }
    /// Inspect the number of callback tasks still owned by this manager.
    pub fn active_handler_tasks(&self) -> usize {
        self.inner.manager.active_handler_tasks()
    }
    /// Forward a received file without decrypting or uploading its bytes.
    pub async fn forward_message_file(
        &self,
        id: i32,
        message: &MessageReceived,
        caption: &str,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.ready()?;
        self.inner
            .manager
            .forward_message_file(id, message, caption, reply_to)
            .await
    }
    /// Receive a file into RAM with an explicit caller-selected plaintext cap.
    pub async fn receive_file_with_limit(
        &self,
        reference: &crate::media::MediaReference,
        maximum: usize,
    ) -> Result<Vec<u8>> {
        self.ready()?;
        self.inner
            .manager
            .receive_file_with_limit(reference, maximum)
            .await
    }
    /// Download a received file to bounded RAM.
    pub async fn receive_message_file(&self, message: &MessageReceived) -> Result<Vec<u8>> {
        self.ready()?;
        self.inner.manager.receive_message_file(message).await
    }
    /// Explicitly save a selected file using the bounded manager policy.
    pub async fn save_file(
        &self,
        reference: &crate::media::MediaReference,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        self.ready()?;
        self.inner.manager.save_file(reference, path).await
    }
    /// Explicitly save a received message's file through the same client.
    pub async fn save_message_file(
        &self,
        message: &MessageReceived,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        self.ready()?;
        self.inner.manager.save_message_file(message, path).await
    }
    /// Withdraw retained message content without deleting relay obligations.
    pub async fn delete_messages(
        &self,
        id: i32,
        random_ids: impl IntoIterator<Item = i64> + Send,
    ) -> Result<()> {
        self.ready()?;
        self.inner.manager.delete_messages(id, random_ids).await
    }
    /// Withdraw conversation content while preserving protected custody.
    pub async fn flush_history(&self, id: i32) -> Result<()> {
        self.ready()?;
        self.inner.manager.flush_history(id).await
    }
    /// Keep automatic plaintext saving disabled for strong ingress.
    pub async fn stop_auto_save_secret_chats(&self) -> Result<()> {
        self.ready()?;
        self.inner.manager.stop_auto_save_secret_chats().await
    }
}
