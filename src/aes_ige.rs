#![forbid(unsafe_code)]

use aes::cipher::{
    BlockCipherDecBackend, BlockCipherDecClosure, BlockCipherDecrypt, BlockCipherEncBackend,
    BlockCipherEncClosure, BlockCipherEncrypt, BlockSizeUser, KeyInit, consts::U16,
};
use aes::{Aes256Dec, Aes256Enc};
use zeroize::Zeroize;

pub fn ige_encrypt(buffer: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
    IgeEncryptor::new(key, iv).encrypt_in_place(buffer);
}

pub fn ige_decrypt(buffer: &mut [u8], key: &[u8; 32], iv: &[u8; 32]) {
    IgeDecryptor::new(key, iv).decrypt_in_place(buffer);
}

pub struct IgeEncryptor {
    cipher: Aes256Enc,
    iv1: u128,
    iv2: u128,
}

pub struct IgeDecryptor {
    cipher: Aes256Dec,
    iv1: u128,
    iv2: u128,
}

impl IgeEncryptor {
    pub fn new(key: &[u8; 32], iv: &[u8; 32]) -> Self {
        let (iv1, iv2) = iv_words(iv);
        Self {
            cipher: Aes256Enc::new(key.into()),
            iv1,
            iv2,
        }
    }

    pub fn encrypt_in_place(&mut self, buffer: &mut [u8]) {
        if let Some(blocks) = blocks(buffer) {
            self.cipher.encrypt_with_backend(Blocks {
                blocks,
                iv1: &mut self.iv1,
                iv2: &mut self.iv2,
            });
        }
    }
}

impl IgeDecryptor {
    pub fn new(key: &[u8; 32], iv: &[u8; 32]) -> Self {
        let (iv1, iv2) = iv_words(iv);
        Self {
            cipher: Aes256Dec::new(key.into()),
            iv1,
            iv2,
        }
    }

    pub fn decrypt_in_place(&mut self, buffer: &mut [u8]) {
        if let Some(blocks) = blocks(buffer) {
            self.cipher.decrypt_with_backend(Blocks {
                blocks,
                iv1: &mut self.iv1,
                iv2: &mut self.iv2,
            });
        }
    }
}

impl Drop for IgeEncryptor {
    fn drop(&mut self) {
        self.iv1.zeroize();
        self.iv2.zeroize();
    }
}

impl Drop for IgeDecryptor {
    fn drop(&mut self) {
        self.iv1.zeroize();
        self.iv2.zeroize();
    }
}

fn iv_words(iv: &[u8; 32]) -> (u128, u128) {
    let (words, _) = iv.as_chunks::<16>();
    (u128::from_ne_bytes(words[0]), u128::from_ne_bytes(words[1]))
}

fn blocks(buffer: &mut [u8]) -> Option<&mut [[u8; 16]]> {
    assert_eq!(
        buffer.len() % 16,
        0,
        "AES-IGE buffer length must be a multiple of 16 bytes"
    );
    let (blocks, _) = buffer.as_chunks_mut();
    (!blocks.is_empty()).then_some(blocks)
}

struct Blocks<'a> {
    blocks: &'a mut [[u8; 16]],
    iv1: &'a mut u128,
    iv2: &'a mut u128,
}

impl BlockSizeUser for Blocks<'_> {
    type BlockSize = U16;
}

impl BlockCipherEncClosure for Blocks<'_> {
    fn call<B: BlockCipherEncBackend<BlockSize = U16>>(self, backend: &B) {
        for block in self.blocks {
            let plain = u128::from_ne_bytes(*block);
            let mut mixed = (plain ^ *self.iv1).to_ne_bytes().into();
            backend.encrypt_block_inplace(&mut mixed);
            let cipher = u128::from_ne_bytes(mixed.into()) ^ *self.iv2;
            *block = cipher.to_ne_bytes();
            *self.iv1 = cipher;
            *self.iv2 = plain;
        }
    }
}

impl BlockCipherDecClosure for Blocks<'_> {
    fn call<B: BlockCipherDecBackend<BlockSize = U16>>(self, backend: &B) {
        for block in self.blocks {
            let cipher = u128::from_ne_bytes(*block);
            let mut mixed = (cipher ^ *self.iv2).to_ne_bytes().into();
            backend.decrypt_block_inplace(&mut mixed);
            let plain = u128::from_ne_bytes(mixed.into()) ^ *self.iv1;
            *block = plain.to_ne_bytes();
            *self.iv1 = cipher;
            *self.iv2 = plain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grammers_crypto::aes as reference;
    use proptest::prelude::*;

    #[test]
    fn unaligned_buffers_match_reference() {
        let key = [0x42; 32];
        let iv = std::array::from_fn(|i| i as u8);
        for len in [0, 16, 32, 48, 256, 4096] {
            let plain: Vec<_> = (0..len).map(|i| i as u8).collect();
            let mut expected = plain.clone();
            reference::ige_encrypt(&mut expected, &key, &iv);
            let mut padded = vec![0x5a; len + 2];
            let block = &mut padded[1..1 + len];
            block.copy_from_slice(&plain);
            ige_encrypt(block, &key, &iv);
            assert_eq!(block, expected);
            reference::ige_decrypt(&mut expected, &key, &iv);
            ige_decrypt(block, &key, &iv);
            assert_eq!(block, expected);
            assert_eq!(block, plain);
            assert_eq!((padded[0], padded[len + 1]), (0x5a, 0x5a));
        }
    }

    #[test]
    #[should_panic(expected = "AES-IGE buffer length must be a multiple of 16 bytes")]
    fn encryption_rejects_partial_blocks() {
        ige_encrypt(&mut [0; 17], &[0; 32], &[0; 32]);
    }

    #[test]
    #[should_panic(expected = "AES-IGE buffer length must be a multiple of 16 bytes")]
    fn decryption_rejects_partial_blocks() {
        ige_decrypt(&mut [0; 15], &[0; 32], &[0; 32]);
    }

    proptest! {
        #[test]
        fn streaming_matches_reference(
            key in any::<[u8; 32]>(),
            iv in any::<[u8; 32]>(),
            chunks in prop::collection::vec(prop::collection::vec(any::<[u8; 16]>(), 0..16), 2..16),
        ) {
            let lengths: Vec<_> = chunks.iter().map(|chunk| chunk.len() * 16).collect();
            let plain: Vec<u8> = chunks.into_iter().flatten().flatten().collect();
            let mut expected = plain.clone();
            reference::ige_encrypt(&mut expected, &key, &iv);
            let mut actual = plain.clone();
            let mut encoder = IgeEncryptor::new(&key, &iv);
            let mut decoder = IgeDecryptor::new(&key, &iv);
            let mut offset = 0;
            for len in lengths {
                let end = offset + len;
                encoder.encrypt_in_place(&mut actual[offset..end]);
                offset = end;
            }
            prop_assert_eq!(&actual, &expected);
            reference::ige_decrypt(&mut expected, &key, &iv);
            for block in actual.as_chunks_mut::<16>().0 {
                decoder.decrypt_in_place(&mut []);
                decoder.decrypt_in_place(block);
            }
            prop_assert_eq!(&actual, &expected);
            prop_assert_eq!(actual, plain);
        }
    }
}
