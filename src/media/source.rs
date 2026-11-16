use crate::error::{Error, Result};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard},
};
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}
pub enum FileSource {
    Path {
        path: PathBuf,
        name: Option<String>,
    },
    Bytes {
        data: Vec<u8>,
        name: String,
    },
    Stream {
        stream: Box<dyn ReadSeek>,
        name: String,
    },
}
pub struct SourceReader {
    pub name: String,
    pub size: u64,
    pub start: u64,
    pub(crate) reader: SharedReader,
}
impl FileSource {
    pub fn path(path: impl AsRef<Path>) -> Self {
        Self::Path {
            path: path.as_ref().to_owned(),
            name: None,
        }
    }
    pub fn bytes(data: Vec<u8>, name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::Media);
        }
        Ok(Self::Bytes { data, name })
    }
    pub fn stream(stream: Box<dyn ReadSeek>, name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::Media);
        }
        Ok(Self::Stream { stream, name })
    }
    pub fn open(self) -> Result<SourceReader> {
        let (mut reader, name): (Box<dyn ReadSeek>, String) = match self {
            Self::Path { path, name } => {
                let name = name
                    .or_else(|| path.file_name().and_then(|n| n.to_str()).map(str::to_owned))
                    .ok_or(Error::Media)?;
                (Box::new(File::open(path).map_err(|_| Error::Media)?), name)
            }
            Self::Bytes { data, name } => (Box::new(Cursor::new(data)), name),
            Self::Stream { stream, name } => (stream, name),
        };
        if name.is_empty() {
            return Err(Error::Media);
        }
        let start = reader.stream_position().map_err(|_| Error::Media)?;
        let end = reader.seek(SeekFrom::End(0)).map_err(|_| Error::Media)?;
        reader
            .seek(SeekFrom::Start(start))
            .map_err(|_| Error::Media)?;
        let size = end.checked_sub(start).ok_or(Error::Media)?;
        Ok(SourceReader {
            name,
            size,
            start,
            reader: SharedReader(Arc::new(SharedCursor {
                reader: Mutex::new(reader),
                coordination: Arc::new(RefillCoord::default()),
            })),
        })
    }
}
impl SourceReader {
    pub(crate) fn encrypting_reader(
        &self,
        key: crate::files::FileKey,
    ) -> Result<super::EncryptingReader<RefillReader>> {
        Ok(
            super::EncryptingReader::new(self.reader.refill_reader(), self.size, key)?
                .with_coordination(self.reader.coordination()),
        )
    }
    pub fn peek(&mut self) -> Result<Vec<u8>> {
        let position = self.reader.stream_position().map_err(|_| Error::Media)?;
        let maximum = usize::try_from(self.size)
            .unwrap_or(usize::MAX)
            .min(crate::ogg::HEADER_BYTES);
        let mut data = Vec::with_capacity(maximum);
        let result = Read::by_ref(&mut self.reader)
            .take(maximum as u64)
            .read_to_end(&mut data);
        self.reader
            .seek(SeekFrom::Start(position))
            .map_err(|_| Error::Media)?;
        result.map_err(|_| Error::Media)?;
        Ok(data)
    }
    pub fn rewind(&mut self) -> Result<()> {
        self.reader
            .seek(SeekFrom::Start(self.start))
            .map_err(|_| Error::Media)?;
        Ok(())
    }
    pub fn reader(&mut self) -> &mut dyn ReadSeek {
        &mut self.reader
    }
}

#[cfg(test)]
#[path = "../../tests/support_media/refill_review.rs"]
mod review;

/// Shared cursor custody lets an owned blocking read outlive its canceled awaiter.
/// Reads already performed advance this same cursor; callers may explicitly rewind.
#[derive(Clone)]
pub(crate) struct SharedReader(Arc<SharedCursor>);
struct SharedCursor {
    reader: Mutex<Box<dyn ReadSeek>>,
    coordination: Arc<RefillCoord>,
}
fn unavailable() -> std::io::Error {
    std::io::Error::other("source is unavailable")
}
impl SharedReader {
    fn coordination(&self) -> Arc<RefillCoord> {
        Arc::clone(&self.0.coordination)
    }
    fn refill_reader(&self) -> RefillReader {
        RefillReader(self.clone())
    }
    fn while_idle<T>(
        &self,
        operation: impl FnOnce(&mut dyn ReadSeek) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let _idle = self.0.coordination.wait_idle()?;
        let mut reader = self.0.reader.lock().map_err(|_| unavailable())?;
        operation(reader.as_mut())
    }
}
impl Read for SharedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.while_idle(|reader| reader.read(bytes))
    }
}
impl Seek for SharedReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.while_idle(|reader| reader.seek(position))
    }
}
/// Only the coordinated encrypting reader uses this view. Its refill lease
/// already excludes other refills and the public cursor operations.
pub(crate) struct RefillReader(SharedReader);
impl Read for RefillReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .0
            .reader
            .lock()
            .map_err(|_| unavailable())?
            .read(bytes)
    }
}
#[derive(Default)]
struct RefillState {
    pending: usize,
    active: bool,
}
#[derive(Default)]
pub(crate) struct RefillCoord {
    state: Mutex<RefillState>,
    idle: Condvar,
}
impl RefillCoord {
    fn wait_idle(&self) -> std::io::Result<MutexGuard<'_, RefillState>> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        while state.pending != 0 {
            state = self.idle.wait(state).map_err(|_| unavailable())?;
        }
        Ok(state)
    }
    pub(crate) fn reserve(self: &Arc<Self>) -> std::io::Result<RefillLease> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        state.pending = state.pending.checked_add(1).ok_or_else(unavailable)?;
        Ok(RefillLease {
            coordination: Arc::clone(self),
            active: false,
        })
    }
}
pub(crate) struct RefillLease {
    coordination: Arc<RefillCoord>,
    active: bool,
}
impl RefillLease {
    pub(crate) fn enter(mut self) -> std::io::Result<Self> {
        let mut state = self.coordination.state.lock().map_err(|_| unavailable())?;
        while state.active {
            state = self
                .coordination
                .idle
                .wait(state)
                .map_err(|_| unavailable())?;
        }
        state.active = true;
        self.active = true;
        drop(state);
        Ok(self)
    }
}
impl Drop for RefillLease {
    fn drop(&mut self) {
        let mut state = self
            .coordination
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.pending -= 1;
        if self.active {
            state.active = false;
        }
        drop(state);
        self.coordination.idle.notify_all();
    }
}
