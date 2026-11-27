//! Layer 234 adapter for the caller's authorized grammers client.

mod grammers;
pub use grammers::GrammersTransport;
pub(crate) use grammers::rejected;

use crate::errors::SecretChatError;
use crate::manager::{BoxFuture, SecretTransport};

type Rpc<'a, T> = BoxFuture<'a, Result<T, SecretChatError>>;
const PART_SIZE: usize = 512 * 1024;
const BIG_FILE_THRESHOLD: usize = 10 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DhConfigData {
    pub g: i32,
    pub p: Vec<u8>,
    pub version: i32,
    pub random: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptedChatWire {
    Waiting {
        chat_id: i32,
        access_hash: i64,
        admin_id: i64,
        participant_id: i64,
        date: i32,
    },
    Requested {
        chat_id: i32,
        access_hash: i64,
        admin_id: i64,
        participant_id: i64,
        g_a: Vec<u8>,
        date: i32,
    },
    Active {
        chat_id: i32,
        access_hash: i64,
        admin_id: i64,
        participant_id: i64,
        g_a_or_b: Vec<u8>,
        key_fingerprint: i64,
        date: i32,
    },
    Discarded {
        chat_id: i32,
        history_deleted: bool,
    },
    Empty {
        chat_id: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingWireUpdate {
    ChatUpdate(EncryptedChatWire),
    NewEncryptedMessage {
        chat_id: i32,
        date: i32,
        bytes: Vec<u8>,
        file: Option<EncryptedFileRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedFileRef {
    pub id: i64,
    pub access_hash: i64,
    pub size: i64,
    pub dc_id: i32,
    pub key_fingerprint: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedFileUpload {
    pub id: i64,
    pub parts: i32,
    pub md5_checksum: String,
    pub key_fingerprint: i32,
    pub is_big: bool,
}

pub trait EstablishmentTransport: SecretTransport + Send + Sync {
    fn get_dh_config(&self, version: i32, random_length: i32) -> Rpc<'_, DhConfigData>;
    fn request_encryption(
        &self,
        peer_user_id: i64,
        peer_access_hash: i64,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> Rpc<'_, EncryptedChatWire>;
    fn accept_encryption(
        &self,
        chat_id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        key_fingerprint: i64,
    ) -> Rpc<'_, EncryptedChatWire>;
    fn discard_encryption(&self, chat_id: i32, delete_history: bool) -> Rpc<'_, bool>;
    fn send_encrypted_file(
        &self,
        chat_id: i32,
        access_hash: i64,
        random_id: i64,
        data: Vec<u8>,
        file: EncryptedFileUpload,
    ) -> Rpc<'_, i32>;
    fn upload_encrypted_bytes(
        &self,
        ciphertext: Vec<u8>,
        key_fingerprint: i32,
    ) -> Rpc<'_, EncryptedFileUpload>;
    fn download_encrypted_bytes(&self, file: &EncryptedFileRef) -> Rpc<'_, Vec<u8>>;
}
