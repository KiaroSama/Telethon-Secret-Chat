//! Lazy block output with Desktop's resume, shrink and vanished-file checks.
use super::error::{ExportError, Result};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
pub struct OutputFile {
    path: PathBuf,
    offset: u64,
    file: Option<std::fs::File>,
}
impl OutputFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            file: None,
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn size(&self) -> u64 {
        self.offset
    }
    pub fn empty(&self) -> bool {
        self.offset == 0
    }
    pub fn close(&mut self) {
        self.file = None;
    }
    fn reopen(&mut self) -> Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        match fs::metadata(&self.path) {
            Ok(meta) if meta.len() < self.offset => return Err(ExportError::Filesystem),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && self.offset > 0 => {
                return Err(ExportError::Filesystem);
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(ExportError::Filesystem);
            }
            _ => {}
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|_| ExportError::Filesystem)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|_| ExportError::Filesystem)?;
        file.set_len(self.offset)
            .map_err(|_| ExportError::Filesystem)?;
        self.file = Some(file);
        Ok(())
    }
    pub fn write_block(&mut self, block: &[u8]) -> Result<()> {
        let result = (|| {
            self.reopen()?;
            if !block.is_empty() {
                let file = self.file.as_mut().ok_or(ExportError::WriterState)?;
                file.write_all(block).map_err(|_| ExportError::Filesystem)?;
                file.flush().map_err(|_| ExportError::Filesystem)?;
                self.offset = self
                    .offset
                    .checked_add(block.len() as u64)
                    .ok_or(ExportError::Filesystem)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
}
