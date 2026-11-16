use crate::error::{Error, Result};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
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
    pub(crate) reader: Box<dyn ReadSeek>,
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
            reader,
        })
    }
}
impl SourceReader {
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
        &mut *self.reader
    }
}
