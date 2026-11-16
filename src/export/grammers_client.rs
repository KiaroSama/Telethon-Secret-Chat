//! Same-client export transport. The caller supplies the authorized client and owns its runner.
use super::{
    error::ExportError,
    fetch_client::*,
    fetch_files::LazyFile,
    model::{FileLocation, kind},
    tl_value::{request_args, to_value},
};
use grammers_client::{Client, InvocationError};
use grammers_tl_types::{self as tl, RemoteCall, Serializable};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
#[derive(Clone)]
pub struct GrammersExportClient {
    client: Client,
}
impl GrammersExportClient {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
    pub fn client(&self) -> &Client {
        &self.client
    }
    async fn invoke<R: RemoteCall>(&self, scope: &RequestScope, request: &R) -> FetchResult<Value>
    where
        R::Return: Serialize,
    {
        let mut call = SerializedCall::<R::Return> {
            bytes: request.to_bytes(),
            marker: std::marker::PhantomData,
        };
        if let Some(range) = &scope.range {
            let range = tl::enums::MessageRange::Range(decode::<tl::types::MessageRange>(range)?);
            call.bytes = tl::functions::InvokeWithMessagesRange {
                range,
                query: SerializedCall::<R::Return> {
                    bytes: call.bytes,
                    marker: std::marker::PhantomData,
                },
            }
            .to_bytes();
        }
        if let Some(takeout_id) = scope.takeout {
            call.bytes = tl::functions::InvokeWithTakeout {
                takeout_id,
                query: SerializedCall::<R::Return> {
                    bytes: call.bytes,
                    marker: std::marker::PhantomData,
                },
            }
            .to_bytes();
        }
        let response = self.client.invoke(&call).await.map_err(error)?;
        to_value(&response).map_err(|_| ExportError::InvalidRecord.into())
    }
}
struct SerializedCall<T> {
    bytes: Vec<u8>,
    marker: std::marker::PhantomData<T>,
}
impl<T> Serializable for SerializedCall<T> {
    fn serialize(&self, sink: &mut impl FnMut(&[u8])) {
        sink(&self.bytes)
    }
}
impl<T: tl::Deserializable> RemoteCall for SerializedCall<T> {
    type Return = T;
}

fn decode<T: DeserializeOwned>(value: &Value) -> FetchResult<T> {
    serde_json::from_value(request_args(value)).map_err(|_| ExportError::InvalidOptions.into())
}
fn error(e: InvocationError) -> FetchError {
    match e {
        InvocationError::Rpc(e) => FetchError::Rpc {
            code: e.code,
            name: e.name.clone(),
            kind: if e.name.starts_with("TAKEOUT_INIT_DELAY") {
                "TakeoutInitDelayError".into()
            } else {
                "RPCError".into()
            },
        },
        _ => ExportError::Filesystem.into(),
    }
}
impl ExportClient for GrammersExportClient {
    fn request<'a>(
        &'a self,
        scope: RequestScope,
        request: Value,
    ) -> BoxFuture<'a, FetchResult<Value>> {
        Box::pin(async move {
            macro_rules! call {
                ($ty:ty) => {{
                    let call = decode::<$ty>(&request)?;
                    self.invoke(&scope, &call).await
                }};
            }
            match kind(&request) {
                "users.GetUsers" => call!(tl::functions::users::GetUsers),
                "help.GetCountriesList" => call!(tl::functions::help::GetCountriesList),
                "account.InitTakeoutSession" => call!(tl::functions::account::InitTakeoutSession),
                "account.FinishTakeoutSession" => {
                    call!(tl::functions::account::FinishTakeoutSession)
                }
                "messages.GetSplitRanges" => call!(tl::functions::messages::GetSplitRanges),
                "messages.GetChats" => call!(tl::functions::messages::GetChats),
                "channels.GetChannels" => call!(tl::functions::channels::GetChannels),
                "channels.GetFullChannel" => call!(tl::functions::channels::GetFullChannel),
                "messages.GetHistory" => call!(tl::functions::messages::GetHistory),
                "messages.Search" => call!(tl::functions::messages::Search),
                "messages.GetMessages" => call!(tl::functions::messages::GetMessages),
                "channels.GetMessages" => call!(tl::functions::channels::GetMessages),
                "messages.GetRichMessage" => call!(tl::functions::messages::GetRichMessage),
                "messages.GetCustomEmojiDocuments" => {
                    call!(tl::functions::messages::GetCustomEmojiDocuments)
                }
                _ => Err(ExportError::Unsupported.into()),
            }
        })
    }
    fn download<'a>(
        &'a self,
        location: &'a FileLocation,
        size: u64,
        target: &'a mut LazyFile,
    ) -> BoxFuture<'a, FetchResult<()>> {
        Box::pin(async move {
            let raw: tl::enums::InputFileLocation = decode(&location.data)?;
            let mut offset = 0i64;
            let mut dc_id = location.dc_id;
            let mut migrations = 0;
            loop {
                let result = self
                    .client
                    .invoke_in_dc(
                        dc_id,
                        &tl::functions::upload::GetFile {
                            precise: true,
                            cdn_supported: false,
                            location: raw.clone(),
                            offset,
                            limit: 128 * 1024,
                        },
                    )
                    .await;
                let result = match result {
                    Ok(result) => result,
                    Err(InvocationError::Rpc(rpc))
                        if rpc.name == "FILE_MIGRATE" && rpc.value.is_some() && migrations < 3 =>
                    {
                        dc_id = i32::try_from(rpc.value.ok_or(ExportError::InvalidRecord)?)
                            .map_err(|_| ExportError::InvalidRecord)?;
                        migrations += 1;
                        continue;
                    }
                    Err(failure) => return Err(error(failure)),
                };
                let tl::enums::upload::File::File(file) = result else {
                    return Err(ExportError::Unsupported.into());
                };
                target.write(&file.bytes)?;
                offset = offset
                    .checked_add(file.bytes.len() as i64)
                    .ok_or(ExportError::InvalidRecord)?;
                if file.bytes.len() < 128 * 1024 || size != 0 && offset as u64 >= size {
                    break;
                }
            }
            Ok(())
        })
    }
}
