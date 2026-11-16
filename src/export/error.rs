//! Fixed error messages exclude conversation contents, source paths and filesystem diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    #[error("Export options are invalid")]
    InvalidOptions,
    #[error("Saved records are invalid or unsupported")]
    InvalidRecord,
    #[error("Saved export failed; saved messages were not changed")]
    Filesystem,
    #[error("Image data is invalid or exceeds the export budget")]
    Image,
    #[error("The export writer is in an invalid state")]
    WriterState,
    #[error("The requested export operation is unsupported")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, ExportError>;
