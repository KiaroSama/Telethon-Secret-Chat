//! Secret-chat media metadata, references and same-client file transport.
mod kind;
mod metadata;
mod reference;
mod source;
mod stream;
mod transfer;

pub(crate) use kind::resolve_kind_named;
pub use kind::{CAPTIONLESS_KINDS, MEDIA_KINDS, MediaKind, guess_mime, resolve_kind};
pub use metadata::{DocumentMediaInput, Metadata, attributes_for, document_media};
pub use reference::{FileParts, MediaReference, file_parts, forwarded_media};
pub use source::{FileSource, ReadSeek, SourceReader};
pub use stream::EncryptingReader;
pub use transfer::{download_bytes, download_file, upload_file};
