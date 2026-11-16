#![allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) use kiaro_secret_chat::export::{
    fetch_client::*,
    fetch_files::*,
    format::DateContext,
    model::*,
    model_message::*,
    settings::{MediaSettings, Settings},
};
pub(super) use serde_json::{Value, json};
pub(super) use std::{collections::VecDeque, fs, sync::Mutex};
#[derive(Default)]
pub(super) struct Fake {
    pub(super) answers: Mutex<VecDeque<FetchResult<Value>>>,
    pub(super) downloads: Mutex<VecDeque<FetchResult<Vec<u8>>>>,
    pub(super) calls: Mutex<Vec<(RequestScope, Value)>>,
    pub(super) downloaded: Mutex<Vec<(FileLocation, u64)>>,
}
impl Fake {
    pub(super) fn scripted(
        answers: Vec<FetchResult<Value>>,
        downloads: Vec<FetchResult<Vec<u8>>>,
    ) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            downloads: Mutex::new(downloads.into()),
            ..Default::default()
        }
    }
}
impl ExportClient for Fake {
    fn request<'a>(
        &'a self,
        scope: RequestScope,
        request: Value,
    ) -> BoxFuture<'a, FetchResult<Value>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((scope, request));
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected request")
        })
    }
    fn download<'a>(
        &'a self,
        location: &'a FileLocation,
        size: u64,
        target: &'a mut LazyFile,
    ) -> BoxFuture<'a, FetchResult<()>> {
        Box::pin(async move {
            self.downloaded
                .lock()
                .unwrap()
                .push((location.clone(), size));
            let bytes = self
                .downloads
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected download")?;
            if !bytes.is_empty() {
                target.write(&bytes)?;
            }
            Ok(())
        })
    }
}
pub(super) fn rpc(name: &str, code: i32) -> FetchError {
    FetchError::Rpc {
        code,
        name: name.into(),
        kind: "RPCError".into(),
    }
}
pub(super) fn location(id: i64, reference: &[u8], thumb: &str, document: bool) -> FileLocation {
    FileLocation {
        dc_id: 2,
        data: json!({"_":if document{"InputDocumentFileLocation"}else{"InputPhotoFileLocation"},"id":id,"access_hash":1,"file_reference":reference,"thumb_size":thumb}),
    }
}
pub(super) fn files<'a>(client: &'a Fake, path: &std::path::Path) -> MessageFiles<'a, Fake> {
    MessageFiles::new(
        client,
        Settings {
            path: path.into(),
            media: MediaSettings {
                types: MediaSettings::ALL_MASK,
                ..Default::default()
            },
            ..Default::default()
        },
        1,
    )
}
pub(super) fn emoji(id: i64, reference: &[u8]) -> Value {
    json!({"_":"Document","id":id,"access_hash":1,"file_reference":reference,"date":1705053605,"mime_type":"image/webp","size":10,"dc_id":2,"attributes":[{"_":"DocumentAttributeSticker","alt":"*"},{"_":"DocumentAttributeFilename","file_name":"e.webp"}]})
}
