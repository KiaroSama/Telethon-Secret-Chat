use super::*;
use crate::{
    Serializable, enums,
    files::FileKey,
    media::{
        FileSource, MediaKind, MediaReference, Metadata, SourceReader, document_media,
        forwarded_media, guess_mime, resolve_kind_named,
    },
    storage::{Operation, StoreMode},
    types,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Clone, Default)]
pub struct SendFileOptions {
    pub caption: String,
    pub mime_type: Option<String>,
    pub kind: Option<MediaKind>,
    pub reply_to: Option<i64>,
    pub metadata: Metadata,
}
impl<S: Storage + 'static, T: Transport> SecretChatManager<S, T> {
    /// Validate kind/metadata before encryption/upload, retaining call order per chat.
    pub async fn send_file(
        &self,
        id: i32,
        source: FileSource,
        options: SendFileOptions,
    ) -> Result<i64> {
        self.with_operation(Some(id), runtime::WorkKind::File, async {
            self.sendable(id)?;
            let mut source = source.open()?;
            let amount = usize::try_from(source.size)
                .unwrap_or(usize::MAX)
                .saturating_mul(4)
                .saturating_add(options.caption.len())
                .saturating_add(options.metadata.thumbnail.as_ref().map_or(0, Vec::len))
                .saturating_add(options.metadata.waveform.as_ref().map_or(0, Vec::len))
                .saturating_add(options.metadata.title.as_ref().map_or(0, String::len))
                .saturating_add(options.metadata.performer.as_ref().map_or(0, String::len))
                .saturating_add(options.metadata.sticker_alt.as_ref().map_or(0, String::len))
                .saturating_add(options.mime_type.as_ref().map_or(0, String::len))
                .saturating_add(1024);
            let _payload = self.reserve_payload(Some(id), amount)?;
            self.ensure_binding().await?;
            let witness = self.witness(id)?;
            let order = self.outbound(id)?;
            let _order = order.lock().await;
            let _headroom = self.reserve_headroom(
                id,
                4,
                self.inner.work.policy.map_or(0, |policy| {
                    policy.frame_bytes.saturating_mul(8).saturating_add(16384)
                }),
            )?;
            self.check_witness(id, witness)?;
            let mime = options
                .mime_type
                .clone()
                .unwrap_or_else(|| guess_mime(&source.name));
            let header = if options.kind.is_none()
                && matches!(mime.as_str(), "audio/ogg" | "audio/opus" | "audio/x-opus")
            {
                source.peek()?
            } else {
                Vec::new()
            };
            let kind =
                resolve_kind_named(&source.name, options.kind, &mime, &options.caption, &header)?;
            options.metadata.validate(kind)?;
            let chat = self.sendable(id)?;
            let layer = crate::framing::outgoing_layer(chat.capability_layer);
            if layer < 143 && source.size >= 1u64 << 31 {
                return Err(Error::Media);
            }
            self.protocol(|p| p.admit(Some(id), Operation::File, 0))?;
            let key = FileKey::generate()?;
            let uploaded = self.inner.transport.upload(&mut source, &key).await?;
            self.check_witness(id, witness)?;
            self.retry_pending_locked(id).await?;
            self.rekey_if_due(id, witness.0).await?;
            let chat = self.sendable(id)?;
            let media = document_media(crate::media::DocumentMediaInput {
                kind,
                name: &source.name,
                mime: &mime,
                size: source.size,
                layer: crate::framing::outgoing_layer(chat.capability_layer),
                key: &key,
                caption: &options.caption,
                metadata: &options.metadata,
            })?;
            let random_id = crate::crypto::random_id()?;
            let message = types::DecryptedMessage {
                no_webpage: false,
                silent: false,
                random_id,
                ttl: chat.ttl,
                message: options.caption,
                media: Some(media),
                entities: None,
                via_bot_name: None,
                reply_to_random_id: options.reply_to,
                grouped_id: None,
            };
            let effect = self
                .protocol(|p| p.prepare_message(id, message.into(), Some(uploaded.to_bytes())))?;
            self.save_sent_file(id, random_id, &effect.outgoing.body, source)
                .await;
            self.transmit(effect, witness.0).await?;
            drop(_order);
            Ok(random_id)
        })
        .await
    }
    /// Reuse a validated server file and its original one-time key without upload.
    pub async fn forward_file(
        &self,
        id: i32,
        reference: &MediaReference,
        caption: &str,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.with_payload_operation(
            Some(id),
            runtime::WorkKind::File,
            reference
                .media_bytes()
                .len()
                .saturating_add(caption.len())
                .saturating_add(256),
            async {
                self.ensure_binding().await?;
                let witness = self.witness(id)?;
                let order = self.outbound(id)?;
                let _order = order.lock().await;
                let _headroom = self.reserve_headroom(
                    id,
                    4,
                    self.inner.work.policy.map_or(0, |policy| {
                        policy.frame_bytes.saturating_mul(8).saturating_add(16384)
                    }),
                )?;
                self.check_witness(id, witness)?;
                self.protocol(|p| p.admit(Some(id), Operation::File, 0))?;
                self.retry_pending_locked(id).await?;
                self.rekey_if_due(id, witness.0).await?;
                let chat = self.sendable(id)?;
                let media = forwarded_media(
                    reference,
                    crate::framing::outgoing_layer(chat.capability_layer),
                    caption,
                )?;
                let random_id = crate::crypto::random_id()?;
                let message = types::DecryptedMessage {
                    no_webpage: false,
                    silent: false,
                    random_id,
                    ttl: chat.ttl,
                    message: caption.into(),
                    media: Some(media),
                    entities: None,
                    via_bot_name: None,
                    reply_to_random_id: reply_to,
                    grouped_id: None,
                };
                let effect = self.protocol(|p| {
                    p.prepare_message(
                        id,
                        message.into(),
                        Some(reference.server_handle().to_bytes()),
                    )
                })?;
                if self
                    .save_outgoing_with(id, &effect.outgoing.body, |autosave| {
                        self.register_owed_file(autosave, id, random_id, reference.clone());
                        Ok(())
                    })
                    .is_err()
                {
                    tracing::warn!(chat_id = id, "Auto-save could not retain a forwarded file");
                }
                self.transmit(effect, witness.0).await?;
                Ok(random_id)
            },
        )
        .await
    }
    /// Forward a received message's file, refusing text-only messages before RPCs.
    pub async fn forward_message_file(
        &self,
        id: i32,
        message: &MessageReceived,
        caption: &str,
        reply_to: Option<i64>,
    ) -> Result<i64> {
        self.with_operation(Some(id), runtime::WorkKind::File, async {
            let reference = MediaReference::new(
                message.chat_id,
                message.file.as_ref().ok_or(Error::Media)?,
                message.media.as_ref().ok_or(Error::Media)?,
            )?;
            self.forward_file(id, &reference, caption, reply_to).await
        })
        .await
    }
    /// Save a received message's file without exposing its one-time key to callers.
    pub async fn save_message_file(
        &self,
        message: &MessageReceived,
        path: &Path,
    ) -> Result<PathBuf> {
        self.with_operation(
            Some(message.chat_id),
            runtime::WorkKind::Persistence,
            async {
                let reference = MediaReference::new(
                    message.chat_id,
                    message.file.as_ref().ok_or(Error::Media)?,
                    message.media.as_ref().ok_or(Error::Media)?,
                )?;
                self.save_file(&reference, path).await
            },
        )
        .await
    }
    /// Download and decrypt a referenced file in bounded RAM, without disk I/O.
    pub async fn receive_file(&self, reference: &MediaReference) -> Result<Vec<u8>> {
        let maximum = self
            .inner
            .work
            .policy
            .map_or(64 * 1024 * 1024, |p| p.bytes / 4);
        self.receive_file_with_limit(reference, maximum).await
    }
    /// Receive with a caller-selected plaintext cap; refusal precedes download.
    pub async fn receive_file_with_limit(
        &self,
        reference: &MediaReference,
        maximum: usize,
    ) -> Result<Vec<u8>> {
        if maximum == 0
            || self
                .inner
                .work
                .policy
                .is_some_and(|policy| maximum > policy.bytes)
        {
            return Err(Error::MediaField { field: "max_bytes" });
        }
        let size = usize::try_from(reference.size()?).map_err(|_| Error::Capacity)?;
        if size > maximum {
            return Err(Error::Paused);
        }
        let amount = size.saturating_mul(4).saturating_add(1024);
        self.with_payload_operation(
            Some(reference.chat_id),
            runtime::WorkKind::File,
            amount,
            async {
                self.ensure_binding().await?;
                self.sendable(reference.chat_id)?;
                self.protocol(|p| {
                    p.admit(
                        Some(reference.chat_id),
                        Operation::File,
                        reference.media_bytes().len(),
                    )
                })?;
                self.inner
                    .transport
                    .download_bytes(reference, maximum)
                    .await
            },
        )
        .await
    }
    /// Receive a message's file in RAM, rechecking its media and attached fingerprint.
    pub async fn receive_message_file(&self, message: &MessageReceived) -> Result<Vec<u8>> {
        let reference = MediaReference::new(
            message.chat_id,
            message.file.as_ref().ok_or(Error::Media)?,
            message.media.as_ref().ok_or(Error::Media)?,
        )?;
        self.receive_file(&reference).await
    }
    /// Download and atomically save a file after validating the reference fingerprint.
    pub async fn save_file(&self, reference: &MediaReference, path: &Path) -> Result<PathBuf> {
        self.with_operation(
            Some(reference.chat_id),
            runtime::WorkKind::Persistence,
            async {
                self.ensure_binding().await?;
                self.generation()?;
                if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
                    return Err(Error::Paused);
                }
                reference.validate()?;
                self.protocol(|p| p.admit(Some(reference.chat_id), Operation::File, 0))?;
                self.inner.transport.download(reference, path).await
            },
        )
        .await
    }
    /// Enable plaintext saved copies. Protected and ingress modes refuse this setting.
    pub async fn start_auto_save_secret_chats(&self, folder: &Path) -> Result<()> {
        self.with_operation(None, runtime::WorkKind::Persistence, async {
            if matches!(
                self.protocol(|p| Ok(p.mode()))?,
                StoreMode::Protected | StoreMode::Ingress
            ) {
                return Err(Error::Paused);
            }
            let (autosave, setting) = AutoSave::enabled(folder)?;
            {
                let mut current = self.inner.autosave.lock().map_err(|_| Error::Storage)?;
                self.protocol(|p| p.set_setting("auto_save_secret_chats".into(), Some(setting)))?;
                *current = autosave;
            }
            self.resume_saved_downloads()
        })
        .await
    }
    /// Close admission, persist OFF, and settle already accepted downloads.
    pub async fn stop_auto_save_secret_chats(&self) -> Result<()> {
        // Cleanup admission is independent of the task slots occupied by the
        // downloads it must join. It performs no new media operation.
        if self.inner.work.policy.is_some() {
            if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
                return Err(Error::Paused);
            }
            self.generation()?;
            if self
                .inner
                .options
                .admission
                .as_ref()
                .is_some_and(|admit| !admit(None, Operation::File))
            {
                self.runtime_refuse(None, crate::storage::RuntimeRefusal::Capacity, false);
                return Err(Error::Paused);
            }
        }
        {
            let mut autosave = self.inner.autosave.lock().map_err(|_| Error::Storage)?;
            let (next, setting) = autosave.disabled();
            self.protocol(|p| p.set_setting("auto_save_secret_chats".into(), Some(setting)))?;
            *autosave = next;
        }
        // The control waiter must not consume a task permit needed by queued
        // downloads, and cancellation must leave their shared join owners intact.
        if let Some(policy) = self.inner.work.policy {
            tokio::time::timeout(
                Duration::from_secs_f64(policy.operation_seconds),
                self.settle_saved_downloads(false),
            )
            .await
            .map_err(|_| Error::ManagerStopping)?
        } else {
            self.settle_saved_downloads(false).await
        }
    }
    /// Return saved records independently of the volatile protocol history cache.
    pub fn read_saved_messages(&self, id: i32) -> Result<Vec<Value>> {
        if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
            return Err(Error::Paused);
        }
        self.inner
            .autosave
            .lock()
            .map_err(|_| Error::Storage)?
            .read(id)
    }
    /// Delete only an explicitly selected saved conversation directory.
    pub fn delete_saved_messages(&self, id: i32) -> Result<()> {
        if self.protocol(|p| Ok(p.mode()))? == StoreMode::Protected {
            return Err(Error::Paused);
        }
        self.inner
            .autosave
            .lock()
            .map_err(|_| Error::Storage)?
            .delete(id)
    }
    /// Active auto-save destination; it contains plaintext and must be protected.
    pub fn auto_save_secret_chats(&self) -> Result<Option<PathBuf>> {
        Ok(self
            .inner
            .autosave
            .lock()
            .map_err(|_| Error::Storage)?
            .folder()
            .map(Path::to_owned))
    }
    pub(crate) fn save_outgoing(&self, id: i32, body: &[u8]) {
        let result = self.save_outgoing_with(id, body, |_| Ok(()));
        if result.is_err() {
            tracing::warn!(chat_id = id, "Auto-save could not append a sent message");
        }
    }
    fn save_outgoing_with(
        &self,
        id: i32,
        body: &[u8],
        after: impl FnOnce(&AutoSave) -> Result<()>,
    ) -> Result<()> {
        (|| {
            let wrapper = crate::framing::unwrap(body)?;
            let plain = crate::autosave::plain_object(body, "DecryptedMessageLayer")?;
            let message = &plain["message"];
            if let Some(action) = message.get("action") {
                self.save_service(id, action, true);
                return Ok(());
            }
            let mut media = message.get("media").cloned().unwrap_or(Value::Null);
            strip_keys(&mut media);
            let record = json!({"v":1,"type":"message","id":wrapper.message.random_id(),"date":now(),"out":true,"text":message.get("message").cloned().unwrap_or_else(||Value::String(String::new())),"entities":message.get("entities").filter(|v|v.is_array()).cloned().unwrap_or_else(||json!([])),"ttl":message.get("ttl").cloned().unwrap_or_else(||json!(0)),"reply_to":message.get("reply_to_random_id").cloned().unwrap_or(Value::Null),"media":media});
            let autosave = self.inner.autosave.lock().map_err(|_| Error::Storage)?;
            autosave.append(id, &record)?;
            after(&autosave)
        })()
    }
    pub(crate) fn save_received(&self, event: &MessageReceived) {
        let result: Result<()> = (|| {
            let mut media = if let Some(media) = &event.media {
                crate::autosave::plain_object(&media.to_bytes(), "DecryptedMessageMedia")?
            } else {
                Value::Null
            };
            strip_keys(&mut media);
            let entities = event
                .entities
                .as_ref()
                .map(|entities| {
                    entities
                        .iter()
                        .map(|entity| {
                            crate::autosave::plain_object(&entity.to_bytes(), "MessageEntity")
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let record = json!({"v":1,"type":"message","id":event.random_id,"date":now(),"out":false,"text":event.text,"entities":entities,"ttl":event.ttl,"reply_to":event.reply_to,"media":media});
            let autosave = self.inner.autosave.lock().map_err(|_| Error::Storage)?;
            autosave.append(event.chat_id, &record)?;
            if let Some(reference) = &event.media_reference {
                self.register_owed_file(
                    &autosave,
                    event.chat_id,
                    event.random_id,
                    reference.clone(),
                );
            }
            Ok(())
        })();
        if result.is_err() {
            tracing::warn!(
                chat_id = event.chat_id,
                "Auto-save could not append a received message"
            );
        }
    }
    pub(crate) fn save_service(&self, id: i32, action: &Value, out: bool) {
        if !matches!(
            action.get("_").and_then(Value::as_str),
            Some(
                "decryptedMessageActionSetMessageTTL"
                    | "decryptedMessageActionScreenshotMessages"
                    | "decryptedMessageActionDeleteMessages"
                    | "decryptedMessageActionFlushHistory"
            )
        ) {
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let record = json!({"v":1,"type":"service","id":format!("service-{nanos}"),"date":now(),"out":out,"action":action});
        if self
            .inner
            .autosave
            .lock()
            .is_ok_and(|autosave| autosave.append(id, &record).is_err())
        {
            tracing::warn!(chat_id = id, "Auto-save could not append a service action");
        }
    }
    async fn save_sent_file(&self, id: i32, random_id: i64, body: &[u8], mut source: SourceReader) {
        let result: Result<()> = async {
            let mut worker = None;
            self.save_outgoing_with(id, body, |autosave| {
                if autosave.folder().is_some() {
                    worker = Some(autosave.worker(id)?);
                }
                Ok(())
            })?;
            let Some(worker) = worker else {
                return Ok(());
            };
            self.inner
                .archive_io
                .scope(async {
                    crate::media::blocking::spawn(move |cancelled| {
                        worker.copy_source(random_id, &mut source, cancelled)
                    })?
                    .await??;
                    Ok(())
                })
                .await
        }
        .await;
        if result.is_err() {
            tracing::warn!(chat_id = id, "Auto-save could not copy a sent file");
        }
    }
    fn register_owed_file(
        &self,
        autosave: &AutoSave,
        id: i32,
        random_id: i64,
        reference: MediaReference,
    ) {
        if autosave.folder().is_none() {
            return;
        }
        let name = reference
            .decoded()
            .ok()
            .map(|media| media_name(&media))
            .unwrap_or_else(|| "file.bin".into());
        if autosave.owe(id, random_id, &reference, &name).is_err() {
            tracing::warn!(chat_id = id, "Auto-save could not retain a file download");
            return;
        }
        self.spawn_saved_download(autosave.clone(), id, random_id, reference, name);
    }
    pub(crate) fn resume_saved_downloads(&self) -> Result<()> {
        let autosave = self.inner.autosave.lock().map_err(|_| Error::Storage)?;
        let owed = match autosave.owed_downloads() {
            Ok(owed) => owed,
            Err(_) => {
                tracing::warn!("Auto-save could not read pending file downloads");
                return Ok(());
            }
        };
        for (id, random_id, reference, name) in owed {
            self.spawn_saved_download(autosave.clone(), id, random_id, reference, name);
        }
        Ok(())
    }
    fn spawn_saved_download(
        &self,
        autosave: AutoSave,
        id: i32,
        random_id: i64,
        reference: MediaReference,
        name: String,
    ) {
        if self
            .queue_saved_download(autosave, id, random_id, reference, name)
            .is_err()
        {
            tracing::warn!(chat_id = id, "Auto-save download admission failed");
        }
    }
}
fn strip_keys(media: &mut Value) {
    if media.get("_").and_then(Value::as_str) == Some("decryptedMessageMediaEmpty") {
        *media = Value::Null;
    } else if let Some(object) = media.as_object_mut() {
        object.remove("key");
        object.remove("iv");
    }
}
fn media_name(media: &enums::DecryptedMessageMedia) -> String {
    let attrs = match media {
        enums::DecryptedMessageMedia::Document(m) => Some(&m.attributes),
        enums::DecryptedMessageMedia::DocumentLegacy7afe8ae2(m) => Some(&m.attributes),
        _ => None,
    };
    if let Some(attrs) = attrs {
        for attribute in attrs {
            if let enums::DocumentAttribute::Filename(name) = attribute
                && !name.file_name.is_empty()
            {
                return crate::autosave::safe_file_name(&name.file_name);
            }
        }
    }
    let mime = crate::media::file_parts(media)
        .ok()
        .and_then(|p| p.mime_type)
        .unwrap_or_default();
    format!(
        "file{}",
        match mime.as_str() {
            "image/jpeg" => ".jpg",
            "video/mp4" => ".mp4",
            "audio/ogg" => ".ogg",
            "audio/mpeg" => ".mp3",
            _ => ".bin",
        }
    )
}
