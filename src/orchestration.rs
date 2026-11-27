//! Caller-owned coordination for secret media, local history, and delivery settlement.

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::{
    autosave::{AutoSaveStore, SavedSecretMessage},
    custody::{CustodyLimits, CustodyMode, CustodyRecord},
    errors::SecretChatError,
    export::{escape_html_text, safe_http_href},
    export_archive::{ExportFormat, ExportOptions, ExportReport, export_saved_chat_with},
    media::{
        EncryptedMediaKey, ForwardMediaReference, MediaAttachmentDescriptor, SecretMediaKind,
        build_forward_media_descriptor, compute_file_key_fingerprint, decrypt_media_payload,
        encrypt_media_payload, parameter, safe_filename, save_media_atomic,
    },
    storage_file::MAX_STORE_BYTES,
    transport::{EncryptedFileRef, EncryptedFileUpload, EstablishmentTransport},
};

/// Per-operation ceilings for both plaintext and block-padded ciphertext.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaBudgets {
    pub max_upload_bytes: usize,
    pub max_download_bytes: usize,
}

impl Default for MediaBudgets {
    fn default() -> Self {
        Self {
            max_upload_bytes: 64 * 1024 * 1024,
            max_download_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct SentEncryptedMedia {
    pub chat_id: i32,
    pub kind: SecretMediaKind,
    pub upload: EncryptedFileUpload,
    pub key: EncryptedMediaKey,
    pub date: i32,
    pub plaintext_size: usize,
}

impl fmt::Debug for SentEncryptedMedia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SentEncryptedMedia")
            .field("chat_id", &self.chat_id)
            .field("kind", &self.kind)
            .field("date", &self.date)
            .field("plaintext_size", &self.plaintext_size)
            .finish_non_exhaustive()
    }
}

/// Coordinates file I/O on a caller-authorized transport. The caller owns protocol
/// frames, sequence state, and retries. A dropped send can leave uploaded parts or
/// an unconfirmed send; it does not roll back RPCs already dispatched.
pub struct MediaOrchestrator<T: EstablishmentTransport> {
    transport: Arc<T>,
    budgets: MediaBudgets,
}

impl<T: EstablishmentTransport> MediaOrchestrator<T> {
    pub fn new(transport: Arc<T>, budgets: MediaBudgets) -> Result<Self, SecretChatError> {
        if budgets.max_upload_bytes == 0 || budgets.max_download_bytes == 0 {
            return Err(parameter("media byte ceilings must be positive"));
        }
        Ok(Self { transport, budgets })
    }

    /// Dispatches an opaque caller-supplied frame. Use `send_encrypted_media_with_frame`
    /// when the frame must include the generated file key and IV.
    #[allow(clippy::too_many_arguments)]
    #[rustfmt::skip]
    pub async fn send_encrypted_media(
        &self, chat_id: i32, access_hash: i64, kind: SecretMediaKind, plaintext: &[u8],
        encrypted_frame: Vec<u8>, random_id: i64,
    ) -> Result<SentEncryptedMedia, SecretChatError> {
        self.send_encrypted_media_with_frame(
            chat_id,
            access_hash,
            kind,
            plaintext,
            random_id,
            |_| Ok(encrypted_frame),
        )
        .await
    }

    /// Builds the caller's encrypted frame with the actual media key, before any RPC.
    /// The frame builder should not commit sequence state: it can run even when the
    /// upload or send subsequently fails or is cancelled.
    #[allow(clippy::too_many_arguments)]
    #[rustfmt::skip]
    pub async fn send_encrypted_media_with_frame(
        &self, chat_id: i32, access_hash: i64, kind: SecretMediaKind, plaintext: &[u8],
        random_id: i64,
        build_frame: impl FnOnce(&EncryptedMediaKey) -> Result<Vec<u8>, SecretChatError>,
    ) -> Result<SentEncryptedMedia, SecretChatError> {
        let plaintext_size = plaintext.len();
        if padded_size(plaintext_size)? > self.budgets.max_upload_bytes {
            return Err(parameter("media exceeds the upload byte ceiling"));
        }
        let (ciphertext, key) = encrypt_media_payload(plaintext)?;
        let frame = build_frame(&key)?;
        let upload = self.transport.upload_encrypted_bytes(ciphertext, key.fingerprint).await?;
        if upload.key_fingerprint != key.fingerprint {
            return Err(parameter(
                "upload returned an unexpected media key fingerprint",
            ));
        }
        let date = self.transport.send_encrypted_file(
            chat_id, access_hash, random_id, frame, upload.clone(),
        ).await?;
        Ok(SentEncryptedMedia {
            chat_id, kind, upload, key, date, plaintext_size,
        })
    }

    /// Checks the declared size before download and the returned length before
    /// decrypting. Telegram's file format has no authentication tag.
    #[rustfmt::skip]
    pub async fn download_and_save_media(
        &self, file: &EncryptedFileRef, expected_size: usize, key: &EncryptedMediaKey,
        dest_dir: &Path, suggested_name: &str,
    ) -> Result<PathBuf, SecretChatError> {
        let padded = padded_size(expected_size)?;
        if padded > self.budgets.max_download_bytes || file.size != padded as i64 {
            return Err(parameter("encrypted file size does not match the expected size or byte ceiling"));
        }
        if file.id == 0 || file.dc_id <= 0 || !safe_filename(suggested_name) {
            return Err(parameter("invalid encrypted media location or filename"));
        }
        if file.key_fingerprint != key.fingerprint
            || compute_file_key_fingerprint(&key.key, &key.iv) != key.fingerprint
        {
            return Err(parameter("encrypted media key fingerprint mismatch"));
        }
        let ciphertext = self.transport.download_encrypted_bytes(file).await?;
        if ciphertext.len() != padded {
            return Err(parameter("download returned an unexpected encrypted file length"));
        }
        let plaintext = Zeroizing::new(decrypt_media_payload(&ciphertext, expected_size, key)?);
        save_media_atomic(
            dest_dir,
            suggested_name,
            &plaintext,
            self.budgets.max_download_bytes,
        )
    }

    /// Reuses the original server file handle; transfers no attachment bytes.
    /// The encrypted frame must describe this same attachment and new caption.
    #[allow(clippy::too_many_arguments)]
    #[rustfmt::skip]
    pub async fn forward_encrypted_media(
        &self, chat_id: i32, access_hash: i64, reference: &ForwardMediaReference,
        new_caption: Option<String>, encrypted_frame: Vec<u8>, random_id: i64,
    ) -> Result<(MediaAttachmentDescriptor, i32), SecretChatError> {
        let descriptor = build_forward_media_descriptor(reference, new_caption)?;
        let size = i64::try_from(descriptor.size_bytes.next_multiple_of(16))
            .map_err(|_| parameter("invalid forwarded media size"))?;
        let file = EncryptedFileRef {
            id: descriptor.file_id, access_hash: descriptor.access_hash, size, dc_id: descriptor.dc_id,
            key_fingerprint: descriptor.key.fingerprint,
        };
        let date = self.transport.send_encrypted_file_reference(
            chat_id, access_hash, random_id, encrypted_frame, file,
        ).await?;
        Ok((descriptor, date))
    }
}

fn padded_size(size: usize) -> Result<usize, SecretChatError> {
    size.checked_next_multiple_of(16)
        .filter(|&padded| (1..=4 * 1024 * 1024 * 1024u64).contains(&(padded as u64)))
        .ok_or_else(|| parameter("invalid encrypted media size"))
}

/// Stores only messages explicitly passed by the caller after protocol processing.
/// No background task or protocol hook is installed. Existing store opt-ins persist.
pub struct AutoSaveSubscription(Mutex<AutoSaveStore>);

impl AutoSaveSubscription {
    #[rustfmt::skip]
    pub fn new(store: AutoSaveStore) -> Self { Self(Mutex::new(store)) }

    pub async fn subscribe_chat(&self, chat_id: i32) -> Result<(), SecretChatError> {
        self.0.lock().await.start_auto_save(chat_id)
    }

    pub async fn unsubscribe_chat(&self, chat_id: i32) -> Result<(), SecretChatError> {
        self.0.lock().await.stop_auto_save(chat_id)
    }

    pub async fn is_subscribed(&self, chat_id: i32) -> bool {
        self.0.lock().await.is_auto_save_enabled(chat_id)
    }

    /// An archive write error is separate from protocol success. Once the lock is
    /// acquired the synchronous store operation runs to completion.
    pub async fn on_message(&self, msg: SavedSecretMessage) -> Result<bool, SecretChatError> {
        self.0.lock().await.record_message(msg)
    }

    pub async fn read_history(&self, chat_id: i32, limit: usize) -> Vec<SavedSecretMessage> {
        self.0.lock().await.read_saved_messages(chat_id, limit)
    }
}

/// Offsets and lengths are UTF-8 byte counts, not Telegram wire UTF-16 offsets.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RichTextEntity {
    Bold { offset: usize, len: usize },
    Italic { offset: usize, len: usize },
    Code { offset: usize, len: usize },
    Pre { offset: usize, len: usize, language: Option<String> },
    TextUrl { offset: usize, len: usize, url: String },
    Mention { offset: usize, len: usize },
    Spoiler { offset: usize, len: usize },
    Strikethrough { offset: usize, len: usize },
}

#[rustfmt::skip]
impl RichTextEntity {
    fn range(&self) -> (usize, usize) {
        use RichTextEntity::*;
        match *self {
            Bold { offset, len } | Italic { offset, len } | Code { offset, len }
            | Pre { offset, len, .. } | TextUrl { offset, len, .. } | Mention { offset, len }
            | Spoiler { offset, len } | Strikethrough { offset, len } => (offset, len),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceArchiveEvent {
    ChatCreated { title: String },
    TtlChanged { seconds: i32 },
    ScreenshotTaken,
    HistoryCleared,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichArchiveRecord {
    pub base: SavedSecretMessage,
    pub entities: Vec<RichTextEntity>,
    pub service_event: Option<ServiceArchiveEvent>,
}

/// Renders disjoint nonempty spans and refuses unsafe link destinations.
#[rustfmt::skip]
pub fn render_rich_html(
    text: &str, entities: &[RichTextEntity],
) -> Result<String, SecretChatError> {
    render_rich(text, entities, None)
}

#[rustfmt::skip]
fn render_rich(
    text: &str, entities: &[RichTextEntity], mut segments: Option<&mut Vec<Value>>,
) -> Result<String, SecretChatError> {
    use RichTextEntity::*;
    let mut ordered: Vec<_> = entities.iter().collect();
    ordered.sort_unstable_by_key(|entity| entity.range());
    let mut out = String::new();
    let mut cursor = 0;
    for entity in ordered {
        let (offset, len) = entity.range();
        let end = offset
            .checked_add(len)
            .filter(|_| len > 0)
            .ok_or_else(|| parameter("invalid archive entity length"))?;
        let plain = text
            .get(cursor..offset)
            .ok_or_else(|| parameter("invalid or overlapping archive entity spans"))?;
        let part = text
            .get(offset..end)
            .ok_or_else(|| parameter("archive entity is outside a UTF-8 text boundary"))?;
        push_plain(&mut out, segments.as_deref_mut(), plain)?;
        let value = escape_html_text(part)?;
        let (kind, html, extra) = match entity {
            Bold { .. } => ("bold", format!("<strong>{value}</strong>"), None),
            Italic { .. } => ("italic", format!("<em>{value}</em>"), None),
            Code { .. } => ("code", format!("<code>{value}</code>"), None),
            Pre { language: None, .. } => ("pre", format!("<pre><code>{value}</code></pre>"), None),
            Pre {
                language: Some(language),
                ..
            } => (
                "pre",
                format!(
                    "<pre><code data-language=\"{}\">{value}</code></pre>",
                    escape_html_text(language)?
                ),
                Some(("language", language.clone())),
            ),
            TextUrl { url, .. } => {
                let href = safe_http_href(url)
                    .ok()
                    .flatten()
                    .ok_or_else(|| parameter("archive entity link must be HTTP or HTTPS"))?;
                (
                    "text_link",
                    format!(
                        "<a href=\"{}\" rel=\"noopener noreferrer\">{value}</a>",
                        escape_html_text(&href)?
                    ),
                    Some(("href", href)),
                )
            }
            Mention { .. } => (
                "mention",
                format!("<span class=\"mention\">{value}</span>"),
                None,
            ),
            Spoiler { .. } => (
                "spoiler",
                format!("<span class=\"spoiler\">{value}</span>"),
                None,
            ),
            Strikethrough { .. } => ("strikethrough", format!("<s>{value}</s>"), None),
        };
        out.push_str(&html);
        if let Some(segments) = segments.as_deref_mut() {
            let mut item = json!({"type": kind, "text": part});
            if let Some((key, value)) = extra {
                item[key] = json!(value);
            }
            segments.push(item);
        }
        cursor = end;
    }
    push_plain(&mut out, segments, &text[cursor..])?;
    Ok(out)
}

#[rustfmt::skip]
fn push_plain(
    out: &mut String, segments: Option<&mut Vec<Value>>, plain: &str,
) -> Result<(), SecretChatError> {
    out.push_str(&escape_html_text(plain)?);
    if let Some(segments) = segments
        && !plain.is_empty()
    {
        segments.push(json!({"type": "plain", "text": plain}));
    }
    Ok(())
}

fn render_service(
    event: &ServiceArchiveEvent,
) -> Result<(&'static str, String, Value), SecretChatError> {
    use ServiceArchiveEvent::*;
    Ok(match event {
        ChatCreated { title } if !title.trim().is_empty() => (
            "chat_created",
            format!("Chat created: {title}"),
            json!({"title": title}),
        ),
        TtlChanged { seconds } if *seconds >= 0 => (
            "ttl_changed",
            format!("TTL changed to {seconds} seconds"),
            json!({"seconds": seconds}),
        ),
        ScreenshotTaken => ("screenshot_taken", "Screenshot taken".into(), json!({})),
        HistoryCleared => ("history_cleared", "History cleared".into(), json!({})),
        _ => return Err(parameter("invalid archive service event")),
    })
}

/// Publishes the base archive and rich sidecars together. The export format
/// selects which `rich_messages` sidecars to write. All output is plaintext.
#[rustfmt::skip]
pub fn export_rich_saved_chat(
    records: &[RichArchiveRecord], options: &ExportOptions,
) -> Result<ExportReport, SecretChatError> {
    let want_html = options.format != ExportFormat::Json;
    let want_json = options.format != ExportFormat::Html;
    let title = escape_html_text(&options.chat_title)?;
    let mut html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"css/style.css\"></head><body><div class=\"page_wrap\"><header class=\"page_header\"><h1 dir=\"auto\">{title}</h1></header><main class=\"page_body chat_page\"><div class=\"history\">\n"
    );
    let mut selected: Vec<_> = records.iter().filter(|record| {
        let msg = &record.base;
        msg.chat_id == options.chat_id
                && options
                    .date_from_unix
                    .is_none_or(|start| msg.date_unix >= start)
                && options.date_to_unix.is_none_or(|end| msg.date_unix <= end)
        })
        .collect();
    selected.sort_unstable_by_key(|record| (record.base.date_unix, record.base.id));
    let mut entries = Vec::new();
    let mut base = Vec::with_capacity(selected.len());
    for record in selected {
        let msg = &record.base;
        let mut segments = Vec::new();
        let content = render_rich(
            &msg.text,
            &record.entities,
            want_json.then_some(&mut segments),
        )?;
        let service = record.service_event.as_ref().map(render_service).transpose()?;
        if want_html {
            let class = if service.is_some() {
                "service"
            } else {
                "default clearfix"
            };
            let sender = escape_html_text(&msg.sender_name)?;
            html.push_str(&format!("<article class=\"message {class}\" id=\"message{}\"><div class=\"body\"><div class=\"from_name\" dir=\"auto\">{sender}</div>", msg.id));
            if let Some((_, label, _)) = &service {
                html.push_str(&format!(
                    "<div class=\"details\" dir=\"auto\">{}</div>",
                    escape_html_text(label)?
                ));
            }
            html.push_str(&format!(
                "<div class=\"text\" dir=\"auto\">{content}</div></div></article>\n"
            ));
        }
        if want_json {
            let mut entry = json!({"id": msg.id, "type": "message", "date_unixtime": msg.date_unix.to_string(), "from": msg.sender_name,
                "from_id": format!("user{}", msg.sender_id), "text": msg.text, "text_entities": segments});
            if let Some((action, _, fields)) = service {
                entry["type"] = json!("service");
                entry["action"] = json!(action);
                entry["action_details"] = fields;
            }
            entries.push(entry);
        }
        base.push(msg.clone());
    }
    html.push_str("</div></main></div></body></html>\n");
    export_saved_chat_with(&base, options, |stage| {
        if want_json {
            let file = fs::File::create(stage.join("rich_messages.json"))
                .map_err(|_| parameter("could not create rich JSON archive"))?;
            let archive = json!({
                "name": options.chat_title, "type": "personal_chat",
                "id": options.peer_id, "messages": entries,
            });
            serde_json::to_writer_pretty(file, &archive)
                .map_err(|_| parameter("could not write rich JSON archive"))?;
        }
        if want_html {
            fs::write(stage.join("rich_messages.html"), html)
                .map_err(|_| parameter("could not write rich HTML archive"))?;
        }
        Ok(())
    })
}

const MAX_TRACKED_CHATS: usize = 4096;
const CAPACITY_PAUSE: &str = "custody capacity reached; retained obligations require settlement";
const MANAGER_PAUSE: &str = "bounded custody manager is paused";
const MISSED_INGRESS: &str = "required ingress was missed while custody was paused";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsettledObligation {
    pub chat_id: i32,
    pub sequence_id: u64,
    pub payload_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementStatus {
    pub chat_id: i32,
    pub unsettled_count: usize,
    pub unsettled_bytes: usize,
    pub paused: bool,
    pub pause_reason: Option<String>,
}

#[derive(Default)]
struct Pause {
    capacity: bool,
    manager: bool,
    missed_ingress: bool,
}

impl Pause {
    #[rustfmt::skip]
    fn reason(&self) -> Option<&'static str> {
        [(self.missed_ingress, MISSED_INGRESS), (self.manager, MANAGER_PAUSE), (self.capacity, CAPACITY_PAUSE)]
            .into_iter().find_map(|(active, reason)| active.then_some(reason))
    }
}

/// Caller-owned in-memory ledger for one bounded manager. Limits apply across chats.
/// Settlement leaves durable raw frames intact and sends no transport acknowledgment.
pub struct BoundedCustodySettlement {
    limits: CustodyLimits,
    pending: BTreeMap<(i32, u64), Zeroizing<Box<[u8]>>>,
    pauses: BTreeMap<i32, Pause>,
    bytes: usize,
}

impl BoundedCustodySettlement {
    pub fn new(limits: CustodyLimits) -> Result<Self, SecretChatError> {
        if !(1..=4096).contains(&limits.max_retained_messages)
            || !(1..=MAX_STORE_BYTES).contains(&limits.max_stored_bytes)
        {
            return Err(parameter("invalid bounded custody limits"));
        }
        Ok(Self {
            limits,
            pending: BTreeMap::new(),
            pauses: BTreeMap::new(),
            bytes: 0,
        })
    }

    /// Identical unsettled replays are harmless. The caller prevents replay
    /// after settlement and must not reuse sequence IDs.
    #[rustfmt::skip]
    pub fn record_obligation(
        &mut self, chat_id: i32, sequence_id: u64, payload: Vec<u8>,
    ) -> Result<(), SecretChatError> {
        let len = payload.len();
        let payload = Zeroizing::new(payload);
        if chat_id == 0 {
            return Err(parameter("invalid bounded custody chat ID"));
        }
        let id = (chat_id, sequence_id);
        if let Some(old) = self.pending.get(&id) {
            if &***old == payload.as_slice() {
                return Ok(());
            }
            return Err(parameter("conflicting custody sequence payload"));
        }
        if let Some(reason) = self.pauses.get(&chat_id).and_then(Pause::reason) {
            return Err(paused(chat_id, reason));
        }
        if self.at_capacity() || len > self.limits.max_stored_bytes - self.bytes {
            self.pause_mut(chat_id)?.capacity = true;
            return Err(paused(chat_id, CAPACITY_PAUSE));
        }
        self.pending.insert(id, Zeroizing::new(payload.as_slice().into()));
        self.bytes += len;
        Ok(())
    }

    /// Returns metadata ordered by sequence ID, including while paused.
    #[rustfmt::skip]
    pub fn poll_unsettled(
        &self, chat_id: i32, offset: usize, limit: usize,
    ) -> Result<Vec<UnsettledObligation>, SecretChatError> {
        if !(1..=100).contains(&limit) {
            return Err(parameter("custody poll limit must be 1 through 100"));
        }
        Ok(self.pending.range((chat_id, 0)..=(chat_id, u64::MAX))
            .skip(offset).take(limit).map(|(&(_, sequence_id), payload)| UnsettledObligation {
                chat_id, sequence_id, payload_bytes: payload.len(),
            }).collect())
    }

    pub fn settle_delivery(
        &mut self,
        chat_id: i32,
        sequence_id: u64,
    ) -> Result<bool, SecretChatError> {
        if let Some(payload) = self.pending.remove(&(chat_id, sequence_id)) {
            self.bytes -= payload.len();
            return Ok(true);
        }
        Ok(false)
    }

    pub fn settlement_status(&self, chat_id: i32) -> SettlementStatus {
        let pending = self.pending.range((chat_id, 0)..=(chat_id, u64::MAX));
        let reason = self.pauses.get(&chat_id).and_then(Pause::reason);
        SettlementStatus {
            chat_id,
            unsettled_count: pending.clone().count(),
            unsettled_bytes: pending.map(|(_, payload)| payload.len()).sum(),
            paused: reason.is_some(),
            pause_reason: reason.map(str::to_owned),
        }
    }

    /// Applies `BoundedSecretChatManager::custody_status`. A missed-ingress
    /// pause stays latched even if a later snapshot is unpaused.
    pub fn observe_manager_status(
        &mut self,
        record: &CustodyRecord,
    ) -> Result<(), SecretChatError> {
        let chat_id = i32::try_from(record.chat_id())
            .map_err(|_| parameter("invalid bounded custody chat ID"))?;
        if chat_id == 0 || record.mode() != CustodyMode::Bounded {
            return Err(parameter("invalid bounded manager record"));
        }
        if record.is_paused() {
            let pause = self.pause_mut(chat_id)?;
            pause.manager = true;
            pause.missed_ingress |= record.pause_reason() == Some(MISSED_INGRESS);
        } else if let Some(pause) = self.pauses.get_mut(&chat_id) {
            pause.manager = false;
            if pause.reason().is_none() {
                self.pauses.remove(&chat_id);
            }
        }
        Ok(())
    }

    /// Clears only a local capacity pause. It never resumes the manager itself.
    pub fn resume_if_under_limits(&mut self, chat_id: i32) -> Result<(), SecretChatError> {
        if let Some(pause) = self.pauses.get(&chat_id)
            && (pause.missed_ingress || pause.manager)
        {
            return Err(paused(chat_id, pause.reason().unwrap_or(MANAGER_PAUSE)));
        }
        if self.at_capacity() {
            return Err(paused(chat_id, CAPACITY_PAUSE));
        }
        self.pauses.remove(&chat_id);
        Ok(())
    }
    #[rustfmt::skip]
    fn at_capacity(&self) -> bool {
        self.pending.len() >= self.limits.max_retained_messages || self.bytes >= self.limits.max_stored_bytes
    }
    fn pause_mut(&mut self, chat_id: i32) -> Result<&mut Pause, SecretChatError> {
        if self.pauses.len() == MAX_TRACKED_CHATS && !self.pauses.contains_key(&chat_id) {
            return Err(parameter("custody pause tracking ceiling reached"));
        }
        Ok(self.pauses.entry(chat_id).or_default())
    }
}

fn paused(chat_id: i32, state: &str) -> SecretChatError {
    SecretChatError::ChatNotReady {
        chat_id: i64::from(chat_id),
        state: state.into(),
    }
}
