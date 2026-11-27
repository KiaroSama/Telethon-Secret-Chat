//! Telegram secret-file encryption, forwarding references, and local downloads.

use std::{
    fs::{self, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};

use grammers_crypto::aes::{ige_decrypt, ige_encrypt};
use grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::{errors::SecretChatError, public_api::CAPTIONLESS_KINDS, storage_file::sync_parent};

const BLOCK: usize = 16;
const CHUNK: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretMediaKind {
    Photo,
    Video,
    Document,
    Audio,
    VoiceNote,
    VideoNote,
    Sticker,
    #[serde(alias = "animation")]
    Gif,
}

impl SecretMediaKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::Document => "document",
            Self::Audio => "audio",
            Self::VoiceNote => "voice_note",
            Self::VideoNote => "video_note",
            Self::Sticker => "sticker",
            Self::Gif => "gif",
        }
    }

    pub fn allows_caption(self) -> bool {
        !CAPTIONLESS_KINDS.contains(&self.as_str())
    }
}

impl FromStr for SecretMediaKind {
    type Err = SecretChatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "photo" => Ok(Self::Photo),
            "video" => Ok(Self::Video),
            "document" => Ok(Self::Document),
            "audio" => Ok(Self::Audio),
            "voice_note" => Ok(Self::VoiceNote),
            "video_note" => Ok(Self::VideoNote),
            "sticker" => Ok(Self::Sticker),
            "gif" | "animation" => Ok(Self::Gif),
            _ => Err(parameter("unrecognized secret media kind")),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaMetadata {
    pub file_name: Option<String>,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_secs: Option<u32>,
    pub title: Option<String>,
    pub performer: Option<String>,
    pub waveform: Option<Vec<u8>>,
    pub sticker_alt: Option<String>,
    pub thumb_bytes: Option<Vec<u8>>,
}

#[derive(Clone, ZeroizeOnDrop)]
pub struct EncryptedMediaKey {
    pub key: [u8; 32],
    pub iv: [u8; 32],
    pub fingerprint: i32,
}

impl std::fmt::Debug for EncryptedMediaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptedMediaKey").finish_non_exhaustive()
    }
}

impl EncryptedMediaKey {
    fn generate() -> Result<Self, SecretChatError> {
        let mut material = Self {
            key: [0; 32],
            iv: [0; 32],
            fingerprint: 0,
        };
        random_fill(&mut material.key)?;
        random_fill(&mut material.iv)?;
        material.fingerprint = compute_file_key_fingerprint(&material.key, &material.iv);
        Ok(material)
    }

    fn verify(&self) -> Result<(), SecretChatError> {
        let actual = compute_file_key_fingerprint(&self.key, &self.iv);
        if bool::from(actual.to_le_bytes().ct_eq(&self.fingerprint.to_le_bytes())) {
            Ok(())
        } else {
            Err(message_rejected("encrypted media key fingerprint mismatch"))
        }
    }
}

/// RFC 1321 MD5. Use only for interoperable legacy fingerprints, not for integrity.
pub fn md5_digest(input: &[u8]) -> [u8; 16] {
    const SHIFT: [[u32; 4]; 4] = [
        [7, 12, 17, 22],
        [5, 9, 14, 20],
        [4, 11, 16, 23],
        [6, 10, 15, 21],
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state = Zeroizing::new([0x67452301_u32, 0xefcdab89, 0x98badcfe, 0x10325476]);
    let mut apply = |block: &[u8; 64]| {
        let mut words = Zeroizing::new([0_u32; 16]);
        for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }
        let [mut a, mut b, mut c, mut d] = *state;
        for (i, k) in K.iter().enumerate() {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let next = a.wrapping_add(f).wrapping_add(*k).wrapping_add(words[g]);
            [a, b, c, d] = [
                d,
                b.wrapping_add(next.rotate_left(SHIFT[i / 16][i % 4])),
                b,
                c,
            ];
        }
        for (word, value) in state.iter_mut().zip([a, b, c, d]) {
            *word = word.wrapping_add(value);
        }
    };
    let (blocks, tail) = input.as_chunks::<64>();
    for block in blocks {
        apply(block);
    }
    let mut last = Zeroizing::new([0_u8; 128]);
    last[..tail.len()].copy_from_slice(tail);
    last[tail.len()] = 0x80;
    let end = if tail.len() < 56 { 64 } else { 128 };
    last[end - 8..end].copy_from_slice(&(input.len() as u64).wrapping_mul(8).to_le_bytes());
    for block in last[..end].as_chunks::<64>().0 {
        apply(block);
    }
    let mut out = [0; 16];
    for (target, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state.iter()) {
        *target = word.to_le_bytes();
    }
    out
}

/// Telegram's secret-file fingerprint is the XOR of the first two little-endian
/// words of MD5(key || iv). It binds only the key and IV, never the file bytes.
pub fn compute_file_key_fingerprint(key: &[u8; 32], iv: &[u8; 32]) -> i32 {
    let mut input = Zeroizing::new([0; 64]);
    input[..32].copy_from_slice(key);
    input[32..].copy_from_slice(iv);
    let digest = Zeroizing::new(md5_digest(&input[..]));
    let words = digest.as_chunks::<4>().0;
    (u32::from_le_bytes(words[0]) ^ u32::from_le_bytes(words[1])) as i32
}

pub fn encrypt_media_payload(
    plaintext: &[u8],
) -> Result<(Vec<u8>, EncryptedMediaKey), SecretChatError> {
    let padded = usize::try_from(padded_size(plaintext.len() as u64)?)
        .map_err(|_| parameter("media payload too large"))?;
    let mut ciphertext = Vec::new();
    ciphertext
        .try_reserve_exact(padded)
        .map_err(|_| parameter("media payload too large"))?;
    let material = encrypt_media_stream(
        &mut Cursor::new(plaintext),
        &mut ciphertext,
        plaintext.len() as u64,
    )?;
    Ok((ciphertext, material))
}

/// Checks the key fingerprint and exact padded size before decrypting. Telegram
/// uses random padding and no file authentication tag, so ciphertext changes of
/// the same length cannot be detected by this operation.
pub fn decrypt_media_payload(
    ciphertext: &[u8],
    expected_size: usize,
    key: &EncryptedMediaKey,
) -> Result<Vec<u8>, SecretChatError> {
    validate_ciphertext(ciphertext.len() as u64, expected_size as u64, key)?;
    let mut plaintext = Vec::new();
    plaintext
        .try_reserve_exact(expected_size)
        .map_err(|_| parameter("media payload too large"))?;
    decrypt_media_stream(
        &mut Cursor::new(ciphertext),
        &mut plaintext,
        ciphertext.len() as u64,
        expected_size as u64,
        key,
    )?;
    Ok(plaintext)
}

/// Encrypts one exact-length source using bounded buffers and fresh OS entropy.
/// On any error the caller must discard the destination, which may be partial.
pub fn encrypt_media_stream(
    source: &mut impl Read,
    destination: &mut impl Write,
    plaintext_size: u64,
) -> Result<EncryptedMediaKey, SecretChatError> {
    padded_size(plaintext_size)?;
    let key = EncryptedMediaKey::generate()?;
    transform(source, destination, plaintext_size, &key, true)?;
    Ok(key)
}

/// Decrypts a Telegram file in bounded chunks. Pass its authenticated message's
/// plaintext size. On any error discard the destination, which may be partial.
pub fn decrypt_media_stream(
    source: &mut impl Read,
    destination: &mut impl Write,
    ciphertext_size: u64,
    expected_size: u64,
    key: &EncryptedMediaKey,
) -> Result<(), SecretChatError> {
    validate_ciphertext(ciphertext_size, expected_size, key)?;
    transform(source, destination, expected_size, key, false)
}

fn padded_size(size: u64) -> Result<u64, SecretChatError> {
    size.checked_add((BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64)
        .ok_or_else(|| parameter("media payload too large"))
}

fn validate_ciphertext(
    size: u64,
    expected_size: u64,
    key: &EncryptedMediaKey,
) -> Result<(), SecretChatError> {
    key.verify()?;
    if size < expected_size
        || !size.is_multiple_of(BLOCK as u64)
        || size - expected_size >= BLOCK as u64
    {
        return Err(message_rejected(
            "invalid encrypted media length or padding size",
        ));
    }
    Ok(())
}

fn transform(
    source: &mut impl Read,
    destination: &mut impl Write,
    plaintext_size: u64,
    key: &EncryptedMediaKey,
    encrypt: bool,
) -> Result<(), SecretChatError> {
    let mut remaining = plaintext_size;
    let mut iv = Zeroizing::new(key.iv);
    let mut previous = Zeroizing::new([0; BLOCK]);
    let mut buffer = Zeroizing::new(vec![0; CHUNK.min(padded_size(remaining)? as usize)]);
    while remaining != 0 {
        let count = remaining.min(CHUNK as u64) as usize;
        let aligned = count.div_ceil(BLOCK) * BLOCK;
        let piece = &mut buffer[..aligned];
        let read = if encrypt { count } else { aligned };
        source
            .read_exact(&mut piece[..read])
            .map_err(|_| message_rejected("could not read complete media payload"))?;
        let last = aligned - BLOCK;
        if encrypt {
            random_fill(&mut piece[count..])?;
            previous.copy_from_slice(&piece[last..]);
            ige_encrypt(piece, &key.key, &iv);
            iv[..BLOCK].copy_from_slice(&piece[last..]);
            iv[BLOCK..].copy_from_slice(&previous[..]);
        } else {
            previous.copy_from_slice(&piece[last..]);
            ige_decrypt(piece, &key.key, &iv);
            iv[..BLOCK].copy_from_slice(&previous[..]);
            iv[BLOCK..].copy_from_slice(&piece[last..]);
        }
        let write = if encrypt { aligned } else { count };
        destination
            .write_all(&piece[..write])
            .map_err(|_| parameter("could not write media payload"))?;
        remaining -= count as u64;
    }
    if source
        .read(&mut [0])
        .map_err(|_| message_rejected("could not check media source length"))?
        != 0
    {
        return Err(message_rejected("media source exceeds its declared length"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct ForwardMediaReference {
    pub kind: SecretMediaKind,
    pub metadata: MediaMetadata,
    pub file_id: i64,
    pub access_hash: i64,
    pub dc_id: i32,
    pub size_bytes: u64,
    pub key: EncryptedMediaKey,
}

#[derive(Clone, Debug)]
pub struct MediaAttachmentDescriptor {
    pub kind: SecretMediaKind,
    pub metadata: MediaMetadata,
    pub file_id: i64,
    pub access_hash: i64,
    pub dc_id: i32,
    pub size_bytes: u64,
    pub key: EncryptedMediaKey,
    pub caption: Option<String>,
}

impl MediaAttachmentDescriptor {
    /// Reuse this handle with `messages.sendEncryptedFile` without uploading.
    pub fn server_handle(&self) -> tl::enums::InputEncryptedFile {
        tl::types::InputEncryptedFile {
            id: self.file_id,
            access_hash: self.access_hash,
        }
        .into()
    }
}

pub fn build_forward_media_descriptor(
    original: &ForwardMediaReference,
    new_caption: Option<String>,
) -> Result<MediaAttachmentDescriptor, SecretChatError> {
    original.key.verify()?;
    let caption = new_caption.filter(|text| !text.is_empty());
    if caption.is_some() && !original.kind.allows_caption() {
        return Err(parameter("this secret media kind does not allow captions"));
    }
    Ok(MediaAttachmentDescriptor {
        kind: original.kind,
        metadata: original.metadata.clone(),
        file_id: original.file_id,
        access_hash: original.access_hash,
        dc_id: original.dc_id,
        size_bytes: original.size_bytes,
        key: original.key.clone(),
        caption,
    })
}

/// Classifies Ogg Opus without music metadata. An incomplete header is not voice.
pub fn detect_voice_note(bytes: &[u8], mime_type: &str) -> bool {
    let mime = mime_type.split(';').next().unwrap_or_default().trim();
    if !["audio/ogg", "audio/opus", "audio/x-opus", "application/ogg"]
        .iter()
        .any(|kind| mime.eq_ignore_ascii_case(kind))
    {
        return false;
    }
    let Some(lacing) = bytes.get(26).map(|&count| 27 + usize::from(count)) else {
        return false;
    };
    bytes
        .get(lacing..)
        .is_some_and(|payload| payload.starts_with(b"OpusHead"))
        && crate::ported::ogg_tags::looks_like_voice(bytes[..bytes.len().min(CHUNK)].to_vec())
            .ok()
            .and_then(|value| value.as_bool())
            == Some(true)
}

pub fn save_media_atomic(
    dest_dir: &Path,
    suggested_name: &str,
    plaintext: &[u8],
    max_bytes: usize,
) -> Result<PathBuf, SecretChatError> {
    if !safe_filename(suggested_name) {
        return Err(parameter("unsafe media filename"));
    }
    if plaintext.len() > max_bytes {
        return Err(parameter("media exceeds the configured size limit"));
    }
    write_media_atomic(dest_dir, suggested_name, |file| file.write_all(plaintext))
        .map_err(|_| parameter("could not save media atomically"))
}

pub(crate) fn safe_filename(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.chars().any(|ch| {
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

pub(crate) fn write_media_atomic(
    dest_dir: &Path,
    name: &str,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| io::Error::from(io::ErrorKind::Other))?;
    let temporary = dest_dir.join(format!(".media-{}.tmp", hex::encode(random)));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let staged = write(&mut file).and_then(|()| file.sync_all());
    drop(file);
    let result = staged.and_then(|()| {
        let path = Path::new(name);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        let extension = path.extension().and_then(|s| s.to_str());
        for suffix in 0..10_000 {
            let candidate = match (suffix, extension) {
                (0, _) => dest_dir.join(name),
                (_, Some(ext)) => dest_dir.join(format!("{stem} ({suffix}).{ext}")),
                _ => dest_dir.join(format!("{stem} ({suffix})")),
            };
            // A portable rename may replace an unrelated file. Publish a complete
            // sibling inode with an atomic link that fails if the name is occupied.
            match fs::hard_link(&temporary, &candidate) {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::from(io::ErrorKind::AlreadyExists))
    });
    let cleanup = fs::remove_file(&temporary);
    let path = result?;
    cleanup?;
    sync_parent(&path)?;
    Ok(path)
}

fn random_fill(bytes: &mut [u8]) -> Result<(), SecretChatError> {
    if bytes.is_empty() {
        return Ok(());
    }
    getrandom::fill(bytes).map_err(|_| SecretChatError::EntropyUnavailable {
        reason: "could not generate secret media randomness".into(),
    })
}

pub(crate) fn parameter(reason: &str) -> SecretChatError {
    SecretChatError::ParameterRejected {
        chat_id: None,
        reason: reason.into(),
    }
}

fn message_rejected(reason: &str) -> SecretChatError {
    SecretChatError::MessageRejected {
        chat_id: None,
        reason: reason.into(),
    }
}
