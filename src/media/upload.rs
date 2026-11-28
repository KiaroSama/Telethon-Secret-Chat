//! Bounded native uploads with an authorization check at every RPC boundary.
use super::{SourceReader, transfer::TransferGuard};
use crate::{
    error::{Error, Result},
    files::FileKey,
};
use grammers_client::Client;
use grammers_tl_types as tl;
use md5::{Digest, Md5};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Mutex,
};

const PART_SIZE: usize = 512 * 1024;
const BIG_FILE_SIZE: usize = 10 * 1024 * 1024;

struct Parts<S> {
    reader: S,
    remaining: usize,
    next: i32,
    checksum: Md5,
}

pub(super) async fn upload(
    client: &Client,
    source: &mut SourceReader,
    key: &FileKey,
    guard: TransferGuard,
) -> Result<tl::enums::InputEncryptedFile> {
    let reader = source.encrypting_reader(key.clone())?;
    let size = usize::try_from(reader.size).map_err(|_| Error::Media)?;
    let total = i32::try_from(size.div_ceil(PART_SIZE)).map_err(|_| Error::Capacity)?;
    let big = size > BIG_FILE_SIZE;
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(|_| Error::Randomness)?;
    let id = i64::from_le_bytes(random);
    let parts = Mutex::new(Parts {
        reader,
        remaining: size,
        next: 0,
        checksum: Md5::new(),
    });
    if big {
        // Match the SDK's four bounded producers while guarding each dispatch.
        tokio::try_join!(
            worker(client, &parts, &guard, id, total, big),
            worker(client, &parts, &guard, id, total, big),
            worker(client, &parts, &guard, id, total, big),
            worker(client, &parts, &guard, id, total, big),
        )?;
    } else {
        worker(client, &parts, &guard, id, total, big).await?;
    }
    guard()?;
    let key_fingerprint = key.fingerprint();
    Ok(if big {
        tl::types::InputEncryptedFileBigUploaded {
            id,
            parts: total,
            key_fingerprint,
        }
        .into()
    } else {
        tl::types::InputEncryptedFileUploaded {
            id,
            parts: total,
            md5_checksum: hex::encode(parts.into_inner().checksum.finalize()),
            key_fingerprint,
        }
        .into()
    })
}

async fn worker<S: AsyncRead + Unpin>(
    client: &Client,
    parts: &Mutex<Parts<S>>,
    guard: &TransferGuard,
    file_id: i64,
    file_total_parts: i32,
    big: bool,
) -> Result<()> {
    loop {
        let (file_part, bytes) = {
            let mut parts = parts.lock().await;
            guard()?;
            if parts.remaining == 0 {
                return Ok(());
            }
            let mut bytes = vec![0; parts.remaining.min(PART_SIZE)];
            let read = parts.reader.read_exact(&mut bytes).await;
            guard()?;
            read.map_err(|_| Error::Transport)?;
            let part = parts.next;
            parts.next += 1;
            parts.remaining -= bytes.len();
            if !big {
                parts.checksum.update(&bytes);
            }
            (part, bytes)
        };
        guard()?;
        let response = if big {
            client
                .invoke(&tl::functions::upload::SaveBigFilePart {
                    file_id,
                    file_part,
                    file_total_parts,
                    bytes,
                })
                .await
        } else {
            client
                .invoke(&tl::functions::upload::SaveFilePart {
                    file_id,
                    file_part,
                    bytes,
                })
                .await
        };
        guard()?;
        if !response.map_err(|_| Error::Transport)? {
            return Err(Error::Transport);
        }
    }
}
