//! Initial secret-chat key exchange and verification pictures.

use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::errors::SecretChatError;
use crate::ported::handshake::fixed_power;
use crate::protocol::{
    ChatRole, ChatState, MAX_ENCRYPTED_FRAME_LEN, SecretChat, check_dh_config, check_peer_value,
    encrypt_frame, key_fingerprint, outgoing_layer, verify_dh_fingerprint, wrap_frame,
};
use crate::transport::{DhConfigData, EncryptedChatWire, EstablishmentTransport, rejected};

pub use crate::public_api::PALETTE;

const KEY_LEN: usize = 256;
const GRID: usize = 12;

pub struct PendingOutgoingHandshake {
    chat_id: i32,
    access_hash: i64,
    admin_id: i64,
    peer_user_id: i64,
    caller_auth_id: String,
    g: i32,
    p: Vec<u8>,
    a: Zeroizing<[u8; KEY_LEN]>,
    g_a: Vec<u8>,
}

impl PendingOutgoingHandshake {
    pub fn chat_id(&self) -> i32 {
        self.chat_id
    }

    pub fn access_hash(&self) -> i64 {
        self.access_hash
    }

    pub fn peer_user_id(&self) -> i64 {
        self.peer_user_id
    }

    pub fn g_a(&self) -> &[u8] {
        &self.g_a
    }
}

impl Drop for PendingOutgoingHandshake {
    fn drop(&mut self) {
        self.a.zeroize();
    }
}

pub struct EstablishedChat {
    pub chat: SecretChat,
    pub g_a: Vec<u8>,
    pub key_picture: KeyPicture,
}

impl EstablishedChat {
    /// Render the retained verification picture, even if the chat's key has rotated.
    pub fn render_key_picture_png(&self, cell_size: u32) -> Result<Vec<u8>, SecretChatError> {
        render_png(&self.key_picture, cell_size)
    }

    /// Prepare a text frame for direct use. Use the manager for retries and service delivery.
    pub fn encrypt_text(
        &mut self,
        random_id: i64,
        text: &str,
        ttl: i32,
    ) -> Result<Vec<u8>, SecretChatError> {
        let chat_id = self.chat.chat_id();
        let id = Some(chat_id);
        if self.chat.state() != ChatState::Ready {
            return Err(SecretChatError::ChatNotReady {
                chat_id,
                state: format!("{:?}", self.chat.state()),
            });
        }
        let seq = self.chat.out_seq_no();
        if random_id == 0
            || ttl < 0
            || text.len() > MAX_ENCRYPTED_FRAME_LEN
            || !(0..=i64::from(i32::MAX) / 2).contains(&seq)
        {
            return Err(rejected(id, "message exceeds a framing bound"));
        }
        let key = Zeroizing::new(
            self.chat
                .export_key_for_internal_frame()
                .ok_or_else(|| rejected(id, "ready chat has no active key"))?,
        );
        let body = Zeroizing::new(wrap_frame(
            &json!({"random_id": random_id, "ttl": ttl, "message": text}),
            outgoing_layer(self.chat.negotiated_layer())?,
            self.chat.in_seq_no() * 2 + i64::from(!self.chat.is_outbound()),
            seq * 2 + i64::from(self.chat.is_outbound()),
        )?);
        let frame = encrypt_frame(key.as_slice(), &body, self.chat.out_x())?;
        if frame.len() > MAX_ENCRYPTED_FRAME_LEN {
            return Err(rejected(
                id,
                "encrypted message exceeds the frame resource limit",
            ));
        }
        self.chat.set_out_seq_no(seq + 1);
        Ok(frame)
    }
}

pub struct SecretChatEstablisher<T: EstablishmentTransport> {
    transport: T,
}

impl<T: EstablishmentTransport> SecretChatEstablisher<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Request a chat and return ownership of its in-memory private exponent.
    ///
    /// Retain this future until it completes. Dropping it after RPC dispatch can
    /// leave a remote request without its local exponent. A timeout that must
    /// remain resumable should retain and resume the same pinned future.
    pub async fn request_chat(
        &self,
        peer_user_id: i64,
        peer_access_hash: i64,
    ) -> Result<PendingOutgoingHandshake, SecretChatError> {
        let caller_auth_id = self.caller_identity()?;
        if peer_user_id <= 0 {
            return Err(rejected(None, "secret chat requires a valid peer user ID"));
        }
        let config = self.dh_config(None).await?;
        let a = private_exponent(&config.random)?;
        let g_a = public_value(config.g, &a, &config.p, None)?;
        let mut random_id = [0; 4];
        fill_random(&mut random_id)?;
        self.require_caller_identity(&caller_auth_id, None)?;
        let waiting = self
            .transport
            .request_encryption(
                peer_user_id,
                peer_access_hash,
                i32::from_le_bytes(random_id),
                g_a.clone(),
            )
            .await?;
        self.require_caller_identity(&caller_auth_id, None)?;
        match waiting {
            EncryptedChatWire::Waiting {
                chat_id,
                access_hash,
                admin_id,
                participant_id,
                ..
            } if chat_id != 0
                && admin_id > 0
                && admin_id != peer_user_id
                && participant_id == peer_user_id =>
            {
                Ok(PendingOutgoingHandshake {
                    chat_id,
                    access_hash,
                    admin_id,
                    peer_user_id,
                    caller_auth_id,
                    g: config.g,
                    p: config.p,
                    a,
                    g_a,
                })
            }
            _ => Err(rejected(
                None,
                "request returned an unexpected secret-chat binding",
            )),
        }
    }

    /// Consume a pending exchange using its matching active-chat update.
    ///
    /// Refusal also consumes and zeroizes the pending exponent. The caller owns
    /// update routing and any required remote discard after a failed exchange.
    pub fn complete_outgoing_handshake(
        &self,
        mut pending: PendingOutgoingHandshake,
        active: &EncryptedChatWire,
    ) -> Result<EstablishedChat, SecretChatError> {
        let chat_id = i64::from(pending.chat_id);
        let id = Some(chat_id);
        let EncryptedChatWire::Active {
            chat_id: active_id,
            access_hash,
            admin_id,
            participant_id,
            g_a_or_b,
            key_fingerprint,
            ..
        } = active
        else {
            return Err(rejected(id, "outgoing secret chat is not active"));
        };
        if *active_id != pending.chat_id
            || *access_hash != pending.access_hash
            || *admin_id != pending.admin_id
            || *participant_id != pending.peer_user_id
            || self.transport.auth_identity() != pending.caller_auth_id
        {
            return Err(rejected(
                id,
                "active secret chat does not match the pending request",
            ));
        }
        let key = shared_key(g_a_or_b, &pending.a, &pending.p, id)?;
        verify_dh_fingerprint(chat_id, key.as_slice(), *key_fingerprint)?;
        let mut chat = SecretChat::new(
            chat_id,
            *access_hash,
            pending.peer_user_id,
            &pending.caller_auth_id,
            ChatRole::Initiator,
        );
        chat.adopt_key(key.as_slice())?;
        chat.configure_dh(i64::from(pending.g), &pending.p)?;
        Ok(EstablishedChat {
            chat,
            g_a: std::mem::take(&mut pending.g_a),
            key_picture: KeyPicture::from_key(&key),
        })
    }

    /// Accept a request while retaining the derived key in this future.
    ///
    /// Retain this future until it completes. Dropping it after RPC dispatch can
    /// leave a remotely accepted chat without its local key. Initial exchanges
    /// are not durable; the caller owns the future and the resulting chat state.
    pub async fn accept_chat(
        &self,
        requested: &EncryptedChatWire,
    ) -> Result<EstablishedChat, SecretChatError> {
        let caller_auth_id = self.caller_identity()?;
        let EncryptedChatWire::Requested {
            chat_id,
            access_hash,
            admin_id,
            participant_id,
            g_a,
            ..
        } = requested
        else {
            return Err(rejected(
                None,
                "accepting a secret chat requires a pending request",
            ));
        };
        let id = Some(i64::from(*chat_id));
        if *chat_id == 0 || *admin_id <= 0 || *participant_id <= 0 || admin_id == participant_id {
            return Err(rejected(
                id,
                "request has invalid secret-chat participants or ID",
            ));
        }
        let config = self.dh_config(id).await?;
        check_wire_value(g_a, &config.p, id)?;
        let b = private_exponent(&config.random)?;
        let g_b = public_value(config.g, &b, &config.p, id)?;
        let key = shared_key(g_a, &b, &config.p, id)?;
        drop(b);
        let fingerprint = key_fingerprint(key.as_slice())?;
        self.require_caller_identity(&caller_auth_id, id)?;
        let active = self
            .transport
            .accept_encryption(*chat_id, *access_hash, g_b, fingerprint)
            .await?;
        self.require_caller_identity(&caller_auth_id, id)?;
        let EncryptedChatWire::Active {
            chat_id: active_id,
            access_hash: active_hash,
            admin_id: active_admin,
            participant_id: active_participant,
            g_a_or_b,
            key_fingerprint: active_fingerprint,
            ..
        } = active
        else {
            return Err(rejected(
                id,
                "accept returned a secret chat that was not active",
            ));
        };
        if active_id != *chat_id
            || active_hash != *access_hash
            || active_admin != *admin_id
            || active_participant != *participant_id
            || g_a_or_b.len() > KEY_LEN
            || !g_a_or_b
                .iter()
                .skip_while(|&&byte| byte == 0)
                .eq(g_a.iter().skip_while(|&&byte| byte == 0))
        {
            return Err(rejected(
                id,
                "accept returned an unexpected secret-chat binding",
            ));
        }
        verify_dh_fingerprint(i64::from(*chat_id), key.as_slice(), active_fingerprint)?;
        let mut chat = SecretChat::new(
            i64::from(*chat_id),
            *access_hash,
            *admin_id,
            caller_auth_id,
            ChatRole::Acceptor,
        );
        chat.adopt_key(key.as_slice())?;
        chat.configure_dh(i64::from(config.g), &config.p)?;
        Ok(EstablishedChat {
            chat,
            g_a: g_a.clone(),
            key_picture: KeyPicture::from_key(&key),
        })
    }

    pub async fn discard_chat(
        &self,
        chat_id: i32,
        delete_history: bool,
    ) -> Result<bool, SecretChatError> {
        EstablishmentTransport::discard_encryption(&self.transport, chat_id, delete_history).await
    }

    pub async fn delete_chat_both_sides(&self, chat_id: i32) -> Result<bool, SecretChatError> {
        self.discard_chat(chat_id, true).await
    }

    fn caller_identity(&self) -> Result<String, SecretChatError> {
        let identity = self.transport.auth_identity();
        if identity.trim().is_empty() {
            return Err(rejected(None, "transport has no authorization binding"));
        }
        Ok(identity.to_owned())
    }

    fn require_caller_identity(
        &self,
        expected: &str,
        chat_id: Option<i64>,
    ) -> Result<(), SecretChatError> {
        if self.transport.auth_identity() != expected {
            return Err(rejected(
                chat_id,
                "caller authorization binding changed during establishment",
            ));
        }
        Ok(())
    }

    async fn dh_config(&self, chat_id: Option<i64>) -> Result<DhConfigData, SecretChatError> {
        let config = self.transport.get_dh_config(0, KEY_LEN as i32).await?;
        if config.p.len() != KEY_LEN || config.random.len() != KEY_LEN {
            return Err(rejected(
                chat_id,
                "DH config requires a 256-byte modulus and random contribution",
            ));
        }
        check_dh_config(i64::from(config.g), &config.p, chat_id)?;
        Ok(config)
    }
}

fn fill_random(bytes: &mut [u8]) -> Result<(), SecretChatError> {
    getrandom::fill(bytes).map_err(|_| SecretChatError::EntropyUnavailable {
        reason: "could not read OS randomness".into(),
    })
}

fn private_exponent(server_random: &[u8]) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretChatError> {
    let mut exponent = Zeroizing::new([0; KEY_LEN]);
    fill_random(exponent.as_mut())?;
    for (local, &remote) in exponent.iter_mut().zip(server_random) {
        *local ^= remote;
    }
    Ok(exponent)
}

fn public_value(
    g: i32,
    secret: &[u8; KEY_LEN],
    p: &[u8],
    chat_id: Option<i64>,
) -> Result<Vec<u8>, SecretChatError> {
    let value = power(&g.to_be_bytes(), secret, p, chat_id)?;
    check_wire_value(&value, p, chat_id)?;
    Ok(value.to_vec())
}

fn shared_key(
    peer: &[u8],
    secret: &[u8; KEY_LEN],
    p: &[u8],
    chat_id: Option<i64>,
) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretChatError> {
    check_wire_value(peer, p, chat_id)?;
    Ok(Zeroizing::new(power(peer, secret, p, chat_id)?))
}

fn check_wire_value(value: &[u8], p: &[u8], chat_id: Option<i64>) -> Result<(), SecretChatError> {
    if value.len() > KEY_LEN {
        return Err(rejected(chat_id, "DH public value exceeds the group width"));
    }
    check_peer_value(value, p, chat_id)
}

fn power(
    base: &[u8],
    exponent: &[u8; KEY_LEN],
    p: &[u8],
    chat_id: Option<i64>,
) -> Result<[u8; KEY_LEN], SecretChatError> {
    fixed_power(base, exponent, p).map_err(|_| rejected(chat_id, "DH exponentiation failed"))
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeyPicture {
    pub rows: [[u8; GRID]; GRID],
    pub hex: String,
    pub sha256_part: [u8; 32],
    pub sha1_part: [u8; 20],
}

impl std::fmt::Debug for KeyPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPicture").finish_non_exhaustive()
    }
}

impl KeyPicture {
    fn from_key(auth_key: &[u8; KEY_LEN]) -> Self {
        let sha1_part: [u8; 20] = Sha1::digest(auth_key).into();
        let sha256_part: [u8; 32] = Sha256::digest(auth_key).into();
        let mut data = [0; 36];
        data[..16].copy_from_slice(&sha1_part[..16]);
        data[16..].copy_from_slice(&sha256_part[..20]);
        let rows = std::array::from_fn(|y| {
            std::array::from_fn(|x| {
                let pixel = y * GRID + x;
                (data[pixel / 4] >> (2 * (pixel % 4))) & 3
            })
        });
        Self {
            rows,
            hex: hex::encode(&data[..32]),
            sha256_part,
            sha1_part,
        }
    }
}

/// The Telegram picture hashes the key, independent of the public exchange value `g_a`.
pub fn render_key_visualization_png(
    auth_key: &[u8; KEY_LEN],
    _g_a: &[u8],
    cell_size: u32,
) -> Result<(KeyPicture, Vec<u8>), SecretChatError> {
    let picture = KeyPicture::from_key(auth_key);
    let png = render_png(&picture, cell_size)?;
    Ok((picture, png))
}

fn render_png(picture: &KeyPicture, cell_size: u32) -> Result<Vec<u8>, SecretChatError> {
    // Bound uncompressed output to about 7 MiB before reserving either buffer.
    if !(1..=128).contains(&cell_size) {
        return Err(rejected(
            None,
            "PNG cell size must be between 1 and 128 pixels",
        ));
    }
    let cell = cell_size as usize;
    let side = GRID * cell;
    let row_bytes = 1 + side * 3;
    let mut raw = Vec::with_capacity(row_bytes * side);
    let mut row = Vec::with_capacity(row_bytes);
    for pixels in &picture.rows {
        row.clear();
        row.push(0);
        for &pixel in pixels {
            let &(r, g, b) = PALETTE
                .get(usize::from(pixel))
                .ok_or_else(|| rejected(None, "key picture has an invalid palette index"))?;
            for _ in 0..cell {
                row.extend_from_slice(&[r, g, b]);
            }
        }
        for _ in 0..cell {
            raw.extend_from_slice(&row);
        }
    }

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = [0; 13];
    let side = (side as u32).to_be_bytes();
    ihdr[..4].copy_from_slice(&side);
    ihdr[4..8].copy_from_slice(&side);
    ihdr[8..10].copy_from_slice(&[8, 2]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &stored_zlib(&raw));
    chunk(&mut png, b"IEND", &[]);
    Ok(png)
}

fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut output = vec![0x78, 0x01];
    let mut blocks = data.chunks(usize::from(u16::MAX)).peekable();
    while let Some(block) = blocks.next() {
        let len = block.len() as u16;
        output.push(u8::from(blocks.peek().is_none()));
        output.extend_from_slice(&len.to_le_bytes());
        output.extend_from_slice(&(!len).to_le_bytes());
        output.extend_from_slice(block);
    }
    let (a, b) = data.iter().fold((1u32, 0u32), |(a, b), &byte| {
        let a = (a + u32::from(byte)) % 65_521;
        (a, (b + a) % 65_521)
    });
    output.extend_from_slice(&((b << 16) | a).to_be_bytes());
    output
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], contents: &[u8]) {
    png.extend_from_slice(&(contents.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(contents);
    let crc = png[start..].iter().fold(u32::MAX, |mut crc, &byte| {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
        crc
    });
    png.extend_from_slice(&(!crc).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_zlib_sets_final_only_on_the_last_block() {
        let data = vec![0; usize::from(u16::MAX) + 1];
        let output = stored_zlib(&data);
        assert_eq!(&output[..7], &[0x78, 0x01, 0, 0xff, 0xff, 0, 0]);
        assert_eq!(
            &output[7 + usize::from(u16::MAX)..],
            &[1, 1, 0, 0xfe, 0xff, 0, 0, 15, 0, 1]
        );
    }
}
