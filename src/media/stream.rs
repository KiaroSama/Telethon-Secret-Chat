use crate::{
    error::{Error, Result},
    files::{FileKey, encrypted_size},
};
use std::{
    future::Future,
    io::{self, Read},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
use zeroize::{Zeroize, Zeroizing};
// Padding and IV state are shared across upload parts. A running blocking
// refill owns its source and zeroizing key copies until the worker is joined.
struct CipherReader<R> {
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
impl<R: Read> CipherReader<R> {
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
impl<R: Read> Read for CipherReader<R> {
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
/// Bounded AES-IGE streaming reader. Async reads execute synchronous sources on
/// the blocking pool; an already running caller Read must cooperate to finish.
pub struct EncryptingReader<R> {
    state: Option<CipherReader<R>>,
    pending: Option<super::blocking::Blocking<(CipherReader<R>, io::Result<()>)>>,
    coordination: Option<std::sync::Arc<super::source::RefillCoord>>,
    pub size: u64,
}
impl<R: Read> EncryptingReader<R> {
    pub fn new(source: R, size: u64, key: FileKey) -> Result<Self> {
        let state = CipherReader::new(source, size, key)?;
        Ok(Self {
            size: state.size,
            state: Some(state),
            pending: None,
            coordination: None,
        })
    }
    /// Use exactly the final block's padding for reproducible wire vectors.
    pub fn new_with_padding(source: R, size: u64, key: FileKey, padding: Vec<u8>) -> Result<Self> {
        let state = CipherReader::new_with_padding(source, size, key, padding)?;
        Ok(Self {
            size: state.size,
            state: Some(state),
            pending: None,
            coordination: None,
        })
    }
    pub(crate) fn with_coordination(
        mut self,
        coordination: std::sync::Arc<super::source::RefillCoord>,
    ) -> Self {
        self.coordination = Some(coordination);
        self
    }
}
impl<R: Read> Read for EncryptingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let state = self.state.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "an asynchronous source read is pending",
            )
        })?;
        let _lease = self
            .coordination
            .as_ref()
            .map(|coordination| coordination.reserve()?.enter())
            .transpose()?;
        state.read(buffer)
    }
}
impl<R: Read + Send + Unpin + 'static> AsyncRead for EncryptingReader<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buffer.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if let Some(pending) = this.pending.as_mut() {
                let (state, result) = match Pin::new(pending).poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(io::Error::other(error))),
                    Poll::Ready(Ok(value)) => value,
                };
                this.pending = None;
                this.state = Some(state);
                if let Err(error) = result {
                    return Poll::Ready(Err(error));
                }
            }
            let Some(state) = this.state.as_mut() else {
                return Poll::Ready(Err(io::Error::other("encrypted source is unavailable")));
            };
            if state.offset < state.ready.len() || (state.left == 0 && state.padding.is_none()) {
                let count = buffer.remaining().min(state.ready.len() - state.offset);
                buffer.put_slice(&state.ready[state.offset..state.offset + count]);
                state.offset += count;
                return Poll::Ready(Ok(()));
            }
            let lease = match this
                .coordination
                .as_ref()
                .map(|coordination| coordination.reserve())
                .transpose()
            {
                Ok(lease) => lease,
                Err(error) => return Poll::Ready(Err(error)),
            };
            let Some(mut state) = this.state.take() else {
                return Poll::Ready(Err(io::Error::other("encrypted source is unavailable")));
            };
            match super::blocking::spawn(move |cancelled| {
                let result = if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "source read was cancelled",
                    ))
                } else {
                    match lease.map(|lease| lease.enter()).transpose() {
                        Ok(active) => {
                            let result = if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                                Err(io::Error::new(
                                    io::ErrorKind::Interrupted,
                                    "source read was cancelled",
                                ))
                            } else {
                                state.refill()
                            };
                            // Release cursor custody in this worker, before
                            // returning state to any asynchronous monitor.
                            drop(active);
                            result
                        }
                        Err(error) => Err(error),
                    }
                };
                (state, result)
            }) {
                Ok(pending) => this.pending = Some(pending),
                Err(error) => return Poll::Ready(Err(io::Error::other(error))),
            }
        }
    }
}
