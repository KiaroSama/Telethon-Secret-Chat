//! File encryption is independent of the chat's shared key. Telegram's file-key
//! fingerprint is a key-binding check, not an integrity tag for file contents.
use crate::{
    crypto::random_bytes,
    error::{Error, Result},
};
use md5::{Digest, Md5};
use std::io::{Read, Write};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct FileKey {
    key: [u8; 32],
    iv: [u8; 32],
}
impl FileKey {
    pub fn generate() -> Result<Self> {
        Ok(Self {
            key: random_bytes()?,
            iv: random_bytes()?,
        })
    }
    pub fn new(key: &[u8], iv: &[u8]) -> Result<Self> {
        Ok(Self {
            key: key.try_into().map_err(|_| Error::Media)?,
            iv: iv.try_into().map_err(|_| Error::Media)?,
        })
    }
    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }
    pub fn iv(&self) -> &[u8; 32] {
        &self.iv
    }
    pub fn fingerprint(&self) -> i32 {
        let mut hash = Md5::new();
        hash.update(self.key);
        hash.update(self.iv);
        let hash = hash.finalize();
        i32::from_le_bytes([
            hash[0] ^ hash[4],
            hash[1] ^ hash[5],
            hash[2] ^ hash[6],
            hash[3] ^ hash[7],
        ])
    }
    pub fn verify(&self, claimed: i32) -> Result<()> {
        if self.fingerprint() != claimed {
            return Err(Error::Fingerprint);
        }
        Ok(())
    }
}
pub fn encrypted_size(size: u64) -> Result<u64> {
    size.checked_add(15)
        .map(|n| n / 16 * 16)
        .ok_or(Error::Media)
}
/// Encrypt exactly `size` bytes from the current stream position. Extra bytes
/// indicate a changing or incorrectly declared source and are refused.
pub fn encrypt_file(
    source: &mut impl Read,
    destination: &mut impl Write,
    size: u64,
    key: &FileKey,
) -> Result<()> {
    let mut iv = Zeroizing::new(key.iv);
    let mut left = size;
    while left > 0 {
        let count = left.min(64 * 1024) as usize;
        let padded = count.div_ceil(16) * 16;
        let mut block = Zeroizing::new(vec![0u8; padded]);
        source
            .read_exact(&mut block[..count])
            .map_err(|_| Error::Media)?;
        if padded > count {
            getrandom::fill(&mut block[count..]).map_err(|_| Error::Randomness)?;
        }
        let mut last_plain = [0; 16];
        last_plain.copy_from_slice(&block[padded - 16..]);
        grammers_crypto::aes::ige_encrypt(&mut block, &key.key, &iv);
        iv[..16].copy_from_slice(&block[padded - 16..]);
        iv[16..].copy_from_slice(&last_plain);
        last_plain.zeroize();
        destination.write_all(&block).map_err(|_| Error::Storage)?;
        left -= count as u64;
    }
    let mut extra = [0; 1];
    if source.read(&mut extra).map_err(|_| Error::Media)? != 0 {
        return Err(Error::Media);
    }
    Ok(())
}
/// Decrypt into a caller-owned sink. Use `save_file` when the destination must
/// remain unchanged on any truncated, extended or failed input.
pub fn decrypt_file(
    source: &mut impl Read,
    destination: &mut impl Write,
    size: u64,
    key: &FileKey,
) -> Result<()> {
    let mut iv = Zeroizing::new(key.iv);
    let mut left = encrypted_size(size)?;
    let mut remaining = size;
    while left > 0 {
        let count = left.min(64 * 1024) as usize;
        let mut block = Zeroizing::new(vec![0; count]);
        source.read_exact(&mut block).map_err(|_| Error::Media)?;
        let mut last_cipher = [0; 16];
        last_cipher.copy_from_slice(&block[count - 16..]);
        grammers_crypto::aes::ige_decrypt(&mut block, &key.key, &iv);
        iv[..16].copy_from_slice(&last_cipher);
        iv[16..].copy_from_slice(&block[count - 16..]);
        last_cipher.zeroize();
        let plain = (remaining.min(count as u64)) as usize;
        destination
            .write_all(&block[..plain])
            .map_err(|_| Error::Storage)?;
        left -= count as u64;
        remaining -= plain as u64;
    }
    let mut extra = [0; 1];
    if source.read(&mut extra).map_err(|_| Error::Media)? != 0 {
        return Err(Error::Media);
    }
    Ok(())
}
pub fn save_file(
    source: &mut impl Read,
    path: &std::path::Path,
    size: u64,
    key: &FileKey,
    claimed: i32,
) -> Result<()> {
    save_file_with_sync(source, path, size, key, claimed, std::fs::File::sync_all)
}
fn save_file_with_sync(
    source: &mut impl Read,
    path: &std::path::Path,
    size: u64,
    key: &FileKey,
    claimed: i32,
    sync: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
) -> Result<()> {
    key.verify(claimed)?;
    let absolute = std::path::absolute(path).map_err(|_| Error::Storage)?;
    let parent = absolute.parent().ok_or(Error::Storage)?;
    crate::temporary::cleanup(parent, ".secret-chat-file-");
    let mut temporary = crate::temporary::create(parent, ".secret-chat-file-")?;
    decrypt_file(source, &mut temporary, size, key)?;
    sync(temporary.as_file()).map_err(|_| Error::Storage)?;
    temporary.persist(&absolute).map_err(|_| Error::Storage)?;
    #[cfg(unix)]
    std::fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error::Storage)?;
    Ok(())
}

#[cfg(test)]
mod primitive_tests {
    use super::*;
    #[test]
    #[allow(clippy::unwrap_used)]
    fn interrupted_media_save_leaves_no_temp_file() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("out.bin");
        let key = FileKey::generate().unwrap();
        let data = b"secret bytes";
        let mut encrypted = Vec::new();
        encrypt_file(&mut &data[..], &mut encrypted, data.len() as u64, &key).unwrap();
        let mut reached = false;
        let failure = save_file_with_sync(
            &mut &encrypted[..],
            &target,
            data.len() as u64,
            &key,
            key.fingerprint(),
            |file| {
                reached = true;
                assert_eq!(file.metadata()?.len(), data.len() as u64);
                Err(std::io::Error::other("synthetic fsync failure"))
            },
        );
        assert!(reached, "actual decrypted staging bytes must reach fsync");
        assert!(matches!(failure, Err(Error::Storage)));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
