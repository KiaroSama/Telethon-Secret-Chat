use crate::{
    error::{Error, Result},
    files::{FileKey, encrypted_size},
};
use std::{
    io::{self, Read},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
use zeroize::{Zeroize, Zeroizing};
/// Bounded streaming AES-IGE reader; padding and IV state are shared across all
/// upload parts. Dropping a cancelled upload releases its source and key copies.
pub struct EncryptingReader<R> {
    source: R,
    key: FileKey,
    iv: Zeroizing<[u8; 32]>,
    left: u64,
    padding: Option<Zeroizing<Vec<u8>>>,
    plain: Zeroizing<Vec<u8>>,
    ready: Zeroizing<Vec<u8>>,
    offset: usize,
    pub size: u64,
}
impl<R: Read> EncryptingReader<R> {
    pub fn new(source: R, size: u64, key: FileKey) -> Result<Self> {
        let encrypted = encrypted_size(size)?;
        let mut padding = vec![0; (encrypted - size) as usize];
        getrandom::fill(&mut padding).map_err(|_| Error::Randomness)?;
        Self::new_with_padding(source, size, key, padding)
    }
    /// Use exactly the final block's padding, e.g. for reproducible wire vectors.
    pub fn new_with_padding(source: R, size: u64, key: FileKey, padding: Vec<u8>) -> Result<Self> {
        let padding = Zeroizing::new(padding);
        let encrypted = encrypted_size(size)?;
        if padding.len() != (encrypted - size) as usize {
            return Err(Error::MediaField { field: "padding" });
        }
        let iv = Zeroizing::new(*key.iv());
        Ok(Self {
            source,
            key,
            iv,
            left: size,
            padding: Some(padding),
            plain: Zeroizing::new(Vec::new()),
            ready: Zeroizing::new(Vec::new()),
            offset: 0,
            size: encrypted,
        })
    }
    fn refill(&mut self) -> io::Result<()> {
        if self.offset < self.ready.len() {
            return Ok(());
        }
        self.ready.clear();
        self.offset = 0;
        while self.ready.is_empty() && (self.left > 0 || self.padding.is_some()) {
            let count = self.left.min(64 * 1024) as usize;
            let mut piece = Zeroizing::new(vec![0; count]);
            let read = if count > 0 {
                self.source.read(&mut piece)?
            } else {
                0
            };
            if self.left > 0 && read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "The media source ended before its declared size",
                ));
            }
            piece.truncate(read);
            self.left -= read as u64;
            self.plain.extend_from_slice(&piece);
            if self.left == 0
                && let Some(mut padding) = self.padding.take()
            {
                self.plain.extend_from_slice(&padding);
                padding.zeroize();
            }
            let whole = self.plain.len() / 16 * 16;
            if whole > 0 {
                let mut block = Zeroizing::new(self.plain.drain(..whole).collect::<Vec<_>>());
                let mut last = [0; 16];
                last.copy_from_slice(&block[whole - 16..]);
                grammers_crypto::aes::ige_encrypt(&mut block, self.key.key(), &self.iv);
                self.iv[..16].copy_from_slice(&block[whole - 16..]);
                self.iv[16..].copy_from_slice(&last);
                last.zeroize();
                self.ready.extend_from_slice(&block);
            }
        }
        Ok(())
    }
}
impl<R: Read> Read for EncryptingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.refill()?;
        let n = buffer.len().min(self.ready.len() - self.offset);
        buffer[..n].copy_from_slice(&self.ready[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}
impl<R: Read + Unpin> AsyncRead for EncryptingReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if let Err(e) = this.refill() {
            return Poll::Ready(Err(e));
        }
        let n = buffer.remaining().min(this.ready.len() - this.offset);
        buffer.put_slice(&this.ready[this.offset..this.offset + n]);
        this.offset += n;
        Poll::Ready(Ok(()))
    }
}
