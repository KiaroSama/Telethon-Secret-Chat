use super::{EncryptingReader, MediaReference, SourceReader};
use crate::{
    error::{Error, Result},
    files::FileKey,
};
use grammers_client::Client;
use grammers_tl_types as tl;
use std::{
    io::{Cursor, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub async fn upload_file(
    client: &Client,
    source: &mut SourceReader,
    key: &FileKey,
) -> Result<tl::enums::InputEncryptedFile> {
    let size = source.size;
    let name = source.name.clone();
    let owned = std::mem::replace(&mut source.reader, Box::new(Cursor::new(Vec::<u8>::new())));
    let mut reader = EncryptingReader::new(owned, size, key.clone())?;
    let encrypted_size = usize::try_from(reader.size).map_err(|_| Error::Media)?;
    let uploaded = client
        .upload_stream(&mut reader, encrypted_size, name)
        .await;
    source.reader = reader.into_source().await.map_err(|_| Error::Storage)?;
    let uploaded = uploaded.map_err(|_| Error::Transport)?;
    let fingerprint = key.fingerprint();
    Ok(match uploaded.raw {
        tl::enums::InputFile::File(file) => tl::types::InputEncryptedFileUploaded {
            id: file.id,
            parts: file.parts,
            md5_checksum: file.md5_checksum,
            key_fingerprint: fingerprint,
        }
        .into(),
        tl::enums::InputFile::Big(file) => tl::types::InputEncryptedFileBigUploaded {
            id: file.id,
            parts: file.parts,
            key_fingerprint: fingerprint,
        }
        .into(),
        _ => return Err(Error::Media),
    })
}
pub async fn download_file(
    client: &Client,
    reference: &MediaReference,
    path: &Path,
) -> Result<PathBuf> {
    let parts = reference.validate()?;
    let absolute = std::path::absolute(path).map_err(|_| Error::Storage)?;
    let parent = absolute.parent().ok_or(Error::Storage)?.to_owned();
    let temporary = super::blocking::spawn(move |cancelled| {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Error::ManagerStopping);
        }
        std::fs::create_dir_all(&parent).map_err(|_| Error::Storage)?;
        tempfile::tempfile_in(parent).map_err(|_| Error::Storage)
    })?
    .await
    .map_err(|_| Error::Storage)??;
    let mut temporary = temporary;
    let mut download = client.iter_download(reference);
    while let Some(bytes) = download.next().await.map_err(|_| Error::Transport)? {
        temporary = super::blocking::spawn(move |cancelled| {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(Error::ManagerStopping);
            }
            temporary.write_all(&bytes).map_err(|_| Error::Storage)?;
            Ok(temporary)
        })?
        .await
        .map_err(|_| Error::Storage)??;
    }
    let fingerprint = reference.key_fingerprint;
    super::blocking::spawn(move |cancelled| {
        temporary.flush().map_err(|_| Error::Storage)?;
        temporary
            .seek(SeekFrom::Start(0))
            .map_err(|_| Error::Storage)?;
        crate::files::save_file_if(
            &mut temporary,
            &absolute,
            parts.size,
            &parts.key,
            fingerprint,
            || {
                if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                    Err(Error::ManagerStopping)
                } else {
                    Ok(())
                }
            },
        )?;
        Ok(absolute)
    })?
    .await
    .map_err(|_| Error::Storage)?
}
/// Receive in bounded RAM; this path never creates a file or temporary directory.
pub async fn download_bytes(
    client: &Client,
    reference: &MediaReference,
    maximum: usize,
) -> Result<Vec<u8>> {
    let parts = reference.validate()?;
    let size = usize::try_from(parts.size).map_err(|_| Error::Capacity)?;
    if size > maximum {
        return Err(Error::Capacity);
    }
    let expected =
        usize::try_from(crate::files::encrypted_size(parts.size)?).map_err(|_| Error::Capacity)?;
    let mut encrypted = zeroize::Zeroizing::new(Vec::with_capacity(expected));
    let mut download = client.iter_download(reference);
    while let Some(bytes) = download.next().await.map_err(|_| Error::Transport)? {
        if encrypted
            .len()
            .checked_add(bytes.len())
            .is_none_or(|total| total > expected)
        {
            return Err(Error::Media);
        }
        encrypted.extend_from_slice(&bytes);
    }
    if encrypted.len() != expected {
        return Err(Error::Media);
    }
    let mut plaintext = Vec::with_capacity(size);
    crate::files::decrypt_file(
        &mut encrypted.as_slice(),
        &mut plaintext,
        parts.size,
        &parts.key,
    )?;
    Ok(plaintext)
}
