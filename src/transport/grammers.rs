use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use grammers_client::{Client, media::Downloadable};
use grammers_tl_types::{
    self as tl,
    functions::{messages as msg, upload},
    types as t,
};
use sha2::{Digest, Sha256};

use crate::errors::SecretChatError;
use crate::manager::SecretTransport;

use super::*;

#[derive(Clone)]
pub struct GrammersTransport {
    client: Client,
    authorization: Authorization,
    identity: String,
    chats: Arc<Mutex<BTreeMap<i32, i64>>>,
    dh: Arc<Mutex<Option<CachedDh>>>,
}

struct CachedDh {
    version: i32,
    g: i32,
    p: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq)]
struct Authorization {
    user_id: i64,
    device_hash: i64,
    dc_id: i32,
}

impl GrammersTransport {
    /// Bind an already authorized client; the caller continues to own its update stream.
    pub async fn new(client: Client, generation: &str) -> Result<Self, SecretChatError> {
        if generation.trim().is_empty() || generation.len() > 128 {
            return Err(rejected(None, "invalid transport generation"));
        }
        let authorization = query_authorization(&client).await?;
        let mut hash = Sha256::new();
        hash.update(b"kiaro-grammers-binding-v1\0");
        hash.update(authorization.user_id.to_le_bytes());
        hash.update(authorization.device_hash.to_le_bytes());
        hash.update(authorization.dc_id.to_le_bytes());
        hash.update(generation.as_bytes());
        Ok(Self {
            client,
            authorization,
            identity: hex::encode(hash.finalize()),
            chats: Arc::default(),
            dh: Arc::default(),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Confirm that the supplied client still belongs to the original account, device and DC.
    pub async fn verified_binding(&self) -> Result<&str, SecretChatError> {
        if query_authorization(&self.client).await? != self.authorization {
            return Err(rejected(None, "caller authorization binding changed"));
        }
        Ok(&self.identity)
    }

    /// Register the access hash when restoring a chat or handling a caller-owned update.
    pub fn register_chat(&self, chat_id: i32, access_hash: i64) -> Result<(), SecretChatError> {
        wire_id(i64::from(chat_id))?;
        let mut chats = self.chats.lock().map_err(|_| cache_error())?;
        if *chats.entry(chat_id).or_insert(access_hash) != access_hash {
            return Err(rejected(
                Some(i64::from(chat_id)),
                "secret-chat access hash changed",
            ));
        }
        Ok(())
    }

    pub async fn set_encrypted_typing(
        &self,
        chat_id: i32,
        access_hash: i64,
        typing: bool,
    ) -> Result<bool, SecretChatError> {
        let peer = peer(chat_id, access_hash);
        self.bound(&msg::SetEncryptedTyping { peer, typing }).await
    }

    pub async fn read_encrypted_history(
        &self,
        chat_id: i32,
        access_hash: i64,
        max_date: i32,
    ) -> Result<bool, SecretChatError> {
        let peer = peer(chat_id, access_hash);
        self.bound(&msg::ReadEncryptedHistory { peer, max_date })
            .await
    }

    pub fn adapt_update(update: &tl::enums::Update) -> Option<IncomingWireUpdate> {
        use tl::enums::{EncryptedMessage, Update};
        let (chat_id, date, bytes, file) = match update {
            Update::Encryption(update) => {
                return Some(IncomingWireUpdate::ChatUpdate((&update.chat).into()));
            }
            Update::NewEncryptedMessage(update) => match &update.message {
                EncryptedMessage::Message(msg) => {
                    (msg.chat_id, msg.date, &msg.bytes, from_file(&msg.file))
                }
                EncryptedMessage::Service(msg) => (msg.chat_id, msg.date, &msg.bytes, None),
            },
            _ => return None,
        };
        Some(IncomingWireUpdate::NewEncryptedMessage {
            chat_id,
            date,
            bytes: bytes.clone(),
            file,
        })
    }

    fn registered_peer(
        &self,
        chat_id: i64,
    ) -> Result<tl::enums::InputEncryptedChat, SecretChatError> {
        let id = wire_id(chat_id)?;
        let chats = self.chats.lock().map_err(|_| cache_error())?;
        let hash = chats
            .get(&id)
            .ok_or(SecretChatError::UnknownChat { chat_id })?;
        Ok(peer(id, *hash))
    }

    async fn bound<R: tl::RemoteCall>(&self, request: &R) -> Result<R::Return, SecretChatError> {
        self.verified_binding().await?;
        self.invoke(request).await
    }

    async fn invoke<R: tl::RemoteCall>(&self, request: &R) -> Result<R::Return, SecretChatError> {
        self.client
            .invoke(request)
            .await
            .map_err(|_| rejected(None, "Telegram RPC failed"))
    }
}

impl SecretTransport for GrammersTransport {
    fn auth_identity(&self) -> &str {
        &self.identity
    }

    fn send_encrypted(&self, chat_id: i64, random_id: i64, data: Vec<u8>) -> Rpc<'_, ()> {
        Box::pin(async move {
            let peer = self.registered_peer(chat_id)?;
            let sent = self
                .bound(&msg::SendEncrypted {
                    silent: false,
                    peer,
                    random_id,
                    data,
                })
                .await?;
            confirm_message(sent, chat_id)
        })
    }

    fn send_encrypted_service(&self, chat_id: i64, random_id: i64, data: Vec<u8>) -> Rpc<'_, ()> {
        Box::pin(async move {
            let peer = self.registered_peer(chat_id)?;
            let sent = self
                .bound(&msg::SendEncryptedService {
                    peer,
                    random_id,
                    data,
                })
                .await?;
            confirm_message(sent, chat_id)
        })
    }

    fn discard_encryption(&self, chat_id: i64, delete_history: bool) -> Rpc<'_, ()> {
        Box::pin(async move {
            if EstablishmentTransport::discard_encryption(self, wire_id(chat_id)?, delete_history)
                .await?
            {
                Ok(())
            } else {
                Err(rejected(
                    Some(chat_id),
                    "server did not discard the secret chat",
                ))
            }
        })
    }
}

impl EstablishmentTransport for GrammersTransport {
    fn get_dh_config(&self, version: i32, random_length: i32) -> Rpc<'_, DhConfigData> {
        Box::pin(async move {
            use tl::enums::messages::DhConfig;
            if !(0..=256).contains(&random_length) || version < 0 {
                return Err(rejected(None, "invalid DH config request"));
            }
            let response = self
                .bound(&msg::GetDhConfig {
                    version,
                    random_length,
                })
                .await?;
            let mut cache = self.dh.lock().map_err(|_| cache_error())?;
            let (g, p, version, random) = match response {
                DhConfig::Config(c) => {
                    if c.p.len() == 256
                        && c.version > 0
                        && cache.as_ref().is_none_or(|old| old.version <= c.version)
                    {
                        *cache = Some(CachedDh {
                            version: c.version,
                            g: c.g,
                            p: c.p.clone(),
                        });
                    }
                    (c.g, c.p, c.version, c.random)
                }
                DhConfig::NotModified(c) => {
                    let cached = cache
                        .as_ref()
                        .filter(|cached| cached.version == version)
                        .ok_or_else(|| rejected(None, "server omitted uncached DH parameters"))?;
                    (cached.g, cached.p.clone(), version, c.random)
                }
            };
            if random.len() != random_length as usize {
                return Err(rejected(
                    None,
                    "server returned unexpected DH entropy length",
                ));
            }
            Ok(DhConfigData {
                g,
                p,
                version,
                random,
            })
        })
    }

    fn request_encryption(
        &self,
        peer_user_id: i64,
        access_hash: i64,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            let user_id = t::InputUser {
                user_id: peer_user_id,
                access_hash,
            }
            .into();
            let reply = self
                .bound(&msg::RequestEncryption {
                    user_id,
                    random_id,
                    g_a,
                })
                .await?;
            let wire = EncryptedChatWire::from(&reply);
            if let EncryptedChatWire::Waiting {
                chat_id,
                access_hash,
                admin_id,
                participant_id,
                ..
            } = wire
            {
                if (admin_id, participant_id) != (self.authorization.user_id, peer_user_id) {
                    return Err(rejected(
                        Some(i64::from(chat_id)),
                        "requested chat returned unexpected participants",
                    ));
                }
                self.register_chat(chat_id, access_hash)?;
            }
            Ok(wire)
        })
    }

    fn accept_encryption(
        &self,
        chat_id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        key_fingerprint: i64,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            let peer = peer(chat_id, access_hash);
            let reply = self
                .bound(&msg::AcceptEncryption {
                    peer,
                    g_b,
                    key_fingerprint,
                })
                .await?;
            let wire = EncryptedChatWire::from(&reply);
            if let EncryptedChatWire::Active {
                chat_id: id,
                access_hash: hash,
                participant_id,
                ..
            } = wire
            {
                if (id, hash, participant_id) != (chat_id, access_hash, self.authorization.user_id)
                {
                    return Err(rejected(
                        Some(i64::from(chat_id)),
                        "accepted chat returned unexpected binding",
                    ));
                }
                self.register_chat(chat_id, access_hash)?;
            }
            Ok(wire)
        })
    }

    fn discard_encryption(&self, chat_id: i32, delete_history: bool) -> Rpc<'_, bool> {
        Box::pin(async move {
            wire_id(i64::from(chat_id))?;
            let discarded = self
                .bound(&msg::DiscardEncryption {
                    chat_id,
                    delete_history,
                })
                .await?;
            if discarded {
                self.chats
                    .lock()
                    .map_err(|_| cache_error())?
                    .remove(&chat_id);
            }
            Ok(discarded)
        })
    }

    fn send_encrypted_file(
        &self,
        chat_id: i32,
        access_hash: i64,
        random_id: i64,
        data: Vec<u8>,
        file: EncryptedFileUpload,
    ) -> Rpc<'_, i32> {
        use tl::enums::messages::SentEncryptedMessage::SentEncryptedFile;
        Box::pin(async move {
            let key_fingerprint = file.key_fingerprint;
            let file = input_upload(file)?;
            let peer = peer(chat_id, access_hash);
            let reply = self
                .bound(&msg::SendEncryptedFile {
                    silent: false,
                    peer,
                    random_id,
                    data,
                    file,
                })
                .await?;
            match reply {
                SentEncryptedFile(sent)
                    if from_file(&sent.file)
                        .is_some_and(|f| f.key_fingerprint == key_fingerprint) =>
                {
                    Ok(sent.date)
                }
                _ => Err(rejected(
                    Some(i64::from(chat_id)),
                    "server did not confirm the encrypted file",
                )),
            }
        })
    }

    fn upload_encrypted_bytes(
        &self,
        ciphertext: Vec<u8>,
        key_fingerprint: i32,
    ) -> Rpc<'_, EncryptedFileUpload> {
        Box::pin(async move {
            check_file_size(ciphertext.len() as u64)?;
            self.verified_binding().await?;
            let mut random = [0; 8];
            getrandom::fill(&mut random).map_err(|_| SecretChatError::EntropyUnavailable {
                reason: "could not generate an upload ID".into(),
            })?;
            let id = i64::from_le_bytes(random);
            if id == 0 {
                return Err(rejected(None, "invalid generated upload ID"));
            }
            let is_big = ciphertext.len() > BIG_FILE_THRESHOLD;
            let parts = ciphertext.len().div_ceil(PART_SIZE) as i32;
            for (part, chunk) in ciphertext.chunks(PART_SIZE).enumerate() {
                let file_id = id;
                let file_part = part as i32;
                let bytes = chunk.to_vec();
                let ok = if is_big {
                    self.invoke(&upload::SaveBigFilePart {
                        file_id,
                        file_part,
                        file_total_parts: parts,
                        bytes,
                    })
                    .await?
                } else {
                    self.invoke(&upload::SaveFilePart {
                        file_id,
                        file_part,
                        bytes,
                    })
                    .await?
                };
                if !ok {
                    return Err(rejected(None, "server rejected an encrypted upload part"));
                }
            }
            let md5_checksum = if is_big {
                String::new()
            } else {
                hex::encode(crate::media::md5_digest(&ciphertext))
            };
            Ok(EncryptedFileUpload {
                id,
                parts,
                md5_checksum,
                key_fingerprint,
                is_big,
            })
        })
    }

    fn download_encrypted_bytes(&self, file: &EncryptedFileRef) -> Rpc<'_, Vec<u8>> {
        let file = file.clone();
        Box::pin(async move {
            let total = check_file_size(file.size as u64)?;
            if file.dc_id <= 0 || file.id == 0 {
                return Err(rejected(None, "invalid encrypted file location"));
            }
            self.verified_binding().await?;
            // The existing client handles upload.getFile and the account's DC routing.
            let mut download = self.client.iter_download(&file);
            let mut bytes = Vec::new();
            while let Some(part) = download
                .next()
                .await
                .map_err(|_| rejected(None, "encrypted file download failed"))?
            {
                if part.len() > total - bytes.len() {
                    return Err(rejected(
                        None,
                        "server returned an invalid encrypted file length",
                    ));
                }
                bytes
                    .try_reserve(part.len())
                    .map_err(|_| rejected(None, "insufficient memory for encrypted file"))?;
                bytes.extend(part);
            }
            if bytes.len() != total {
                return Err(rejected(None, "encrypted file download was truncated"));
            }
            Ok(bytes)
        })
    }
}

impl Downloadable for EncryptedFileRef {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            t::InputEncryptedFileLocation {
                id: self.id,
                access_hash: self.access_hash,
            }
            .into(),
        )
    }

    fn size(&self) -> Option<usize> {
        usize::try_from(self.size).ok()
    }

    fn dc_id(&self) -> Option<i32> {
        Some(self.dc_id)
    }
}

impl From<&tl::enums::EncryptedChat> for EncryptedChatWire {
    fn from(chat: &tl::enums::EncryptedChat) -> Self {
        use tl::enums::EncryptedChat as Tl;
        match chat {
            Tl::Waiting(c) => Self::Waiting {
                chat_id: c.id,
                access_hash: c.access_hash,
                admin_id: c.admin_id,
                participant_id: c.participant_id,
                date: c.date,
            },
            Tl::Requested(c) => Self::Requested {
                chat_id: c.id,
                access_hash: c.access_hash,
                admin_id: c.admin_id,
                participant_id: c.participant_id,
                g_a: c.g_a.clone(),
                date: c.date,
            },
            Tl::Chat(c) => Self::Active {
                chat_id: c.id,
                access_hash: c.access_hash,
                admin_id: c.admin_id,
                participant_id: c.participant_id,
                g_a_or_b: c.g_a_or_b.clone(),
                key_fingerprint: c.key_fingerprint,
                date: c.date,
            },
            Tl::Discarded(c) => Self::Discarded {
                chat_id: c.id,
                history_deleted: c.history_deleted,
            },
            Tl::Empty(c) => Self::Empty { chat_id: c.id },
        }
    }
}

fn from_file(file: &tl::enums::EncryptedFile) -> Option<EncryptedFileRef> {
    let tl::enums::EncryptedFile::File(f) = file else {
        return None;
    };
    Some(EncryptedFileRef {
        id: f.id,
        access_hash: f.access_hash,
        size: f.size,
        dc_id: f.dc_id,
        key_fingerprint: f.key_fingerprint,
    })
}

fn input_upload(
    file: EncryptedFileUpload,
) -> Result<tl::enums::InputEncryptedFile, SecretChatError> {
    let EncryptedFileUpload {
        id,
        parts,
        md5_checksum,
        key_fingerprint,
        is_big,
    } = file;
    if id == 0 || parts <= 0 {
        return Err(rejected(None, "encrypted upload has no file ID or parts"));
    }
    if is_big {
        return Ok(t::InputEncryptedFileBigUploaded {
            id,
            parts,
            key_fingerprint,
        }
        .into());
    }
    if md5_checksum.len() != 32 || !md5_checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(rejected(
            None,
            "small encrypted upload has an invalid checksum",
        ));
    }
    Ok(t::InputEncryptedFileUploaded {
        id,
        parts,
        md5_checksum,
        key_fingerprint,
    }
    .into())
}

fn peer(chat_id: i32, access_hash: i64) -> tl::enums::InputEncryptedChat {
    t::InputEncryptedChat {
        chat_id,
        access_hash,
    }
    .into()
}

fn wire_id(chat_id: i64) -> Result<i32, SecretChatError> {
    i32::try_from(chat_id)
        .ok()
        .filter(|id| *id != 0)
        .ok_or_else(|| rejected(Some(chat_id), "invalid Telegram secret chat ID"))
}

fn confirm_message(
    reply: tl::enums::messages::SentEncryptedMessage,
    chat_id: i64,
) -> Result<(), SecretChatError> {
    if matches!(reply, tl::enums::messages::SentEncryptedMessage::Message(_)) {
        Ok(())
    } else {
        Err(rejected(
            Some(chat_id),
            "server returned an unexpected encrypted send result",
        ))
    }
}

fn check_file_size(size: u64) -> Result<usize, SecretChatError> {
    if size == 0 || size > MAX_FILE_BYTES || !size.is_multiple_of(16) {
        return Err(rejected(None, "invalid encrypted file size"));
    }
    usize::try_from(size).map_err(|_| rejected(None, "invalid encrypted file size"))
}

async fn query_authorization(client: &Client) -> Result<Authorization, SecretChatError> {
    use tl::{
        enums as e,
        functions::{account, help, users},
    };
    let invalid = || {
        rejected(
            None,
            "caller client has no verified Telegram user authorization",
        )
    };
    let users = client
        .invoke(&users::GetUsers {
            id: vec![e::InputUser::UserSelf],
        })
        .await
        .map_err(|_| invalid())?;
    let [e::User::User(user)] = users.as_slice() else {
        return Err(invalid());
    };
    if !user.is_self || user.bot || user.id <= 0 {
        return Err(invalid());
    }
    let e::account::Authorizations::Authorizations(auths) = client
        .invoke(&account::GetAuthorizations {})
        .await
        .map_err(|_| invalid())?;
    let mut current = auths.authorizations.iter().filter_map(|entry| {
        let e::Authorization::Authorization(entry) = entry;
        entry.current.then_some(entry)
    });
    let device = current.next().ok_or_else(invalid)?;
    if current.next().is_some()
        || device.hash == 0
        || device.password_pending
        || device.unconfirmed
        || device.encrypted_requests_disabled
    {
        return Err(invalid());
    }
    let e::Config::Config(config) = client
        .invoke(&help::GetConfig {})
        .await
        .map_err(|_| invalid())?;
    if !(1..=1000).contains(&config.this_dc) {
        return Err(invalid());
    }
    Ok(Authorization {
        user_id: user.id,
        device_hash: device.hash,
        dc_id: config.this_dc,
    })
}

fn cache_error() -> SecretChatError {
    rejected(None, "transport cache unavailable")
}

pub(crate) fn rejected(chat_id: Option<i64>, reason: &'static str) -> SecretChatError {
    SecretChatError::ParameterRejected {
        chat_id,
        reason: reason.into(),
    }
}
