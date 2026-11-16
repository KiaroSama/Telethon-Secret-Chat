//! Inspect every wrapper in the pinned TL schema before acknowledging qts.
use crate::error::{Error, Result};
use grammers_tl_types::{self as tl, Deserializable, Identifiable};
pub(super) enum Request {
    Difference(tl::functions::updates::GetDifference),
    Channel(tl::functions::updates::GetChannelDifference),
    Queue(i32),
    State,
    Ping(i64),
    Other,
}
struct Tail(Vec<u8>);
impl Deserializable for Tail {
    fn deserialize(cursor: &mut tl::deserialize::Cursor<'_>) -> tl::deserialize::Result<Self> {
        let mut body = vec![];
        cursor.read_to_end(&mut body)?;
        Ok(Self(body))
    }
}
pub(super) fn decode<T: Deserializable>(bytes: &[u8]) -> Result<T> {
    let mut cursor = tl::deserialize::Cursor::from_slice(bytes);
    let value = T::deserialize(&mut cursor).map_err(|_| Error::IngressUnavailable)?;
    if cursor.pos() != bytes.len() {
        return Err(Error::IngressUnavailable);
    }
    Ok(value)
}
fn flags(args: &[u8], known: u32) -> Result<()> {
    let flags = u32::from_le_bytes(
        args.get(..4)
            .ok_or(Error::IngressUnavailable)?
            .try_into()
            .map_err(|_| Error::IngressUnavailable)?,
    );
    if flags & !known != 0 {
        Err(Error::IngressUnavailable)
    } else {
        Ok(())
    }
}
pub(super) fn inspect(body: &[u8]) -> Result<Request> {
    if body.len() > 4 * 1024 * 1024 {
        return Err(Error::Capacity);
    }
    let mut body = body.to_vec();
    for _ in 0..8 {
        let id = u32::from_le_bytes(
            body.get(..4)
                .ok_or(Error::IngressUnavailable)?
                .try_into()
                .map_err(|_| Error::IngressUnavailable)?,
        );
        let args = &body[4..];
        macro_rules! wrapper {
            ($typ:ident) => {
                decode::<tl::functions::$typ<Tail>>(args)?.query.0
            };
        }
        body = match id {
            id if id == tl::functions::InvokeWithoutUpdates::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithoutUpdates)
            }
            id if id == tl::functions::InvokeWithLayer::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithLayer)
            }
            id if id == tl::functions::InvokeAfterMsg::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeAfterMsg)
            }
            id if id == tl::functions::InvokeAfterMsgs::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeAfterMsgs)
            }
            id if id == tl::functions::InvokeWithTakeout::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithTakeout)
            }
            id if id == tl::functions::InitConnection::<Tail>::CONSTRUCTOR_ID => {
                flags(args, 3)?;
                wrapper!(InitConnection)
            }
            id if id == tl::functions::InvokeWithMessagesRange::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithMessagesRange)
            }
            id if id == tl::functions::InvokeWithBusinessConnection::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithBusinessConnection)
            }
            id if id == tl::functions::InvokeWithGooglePlayIntegrity::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithGooglePlayIntegrity)
            }
            id if id == tl::functions::InvokeWithApnsSecret::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithApnsSecret)
            }
            id if id == tl::functions::InvokeWithReCaptcha::<Tail>::CONSTRUCTOR_ID => {
                wrapper!(InvokeWithReCaptcha)
            }
            id if id == tl::functions::updates::GetDifference::CONSTRUCTOR_ID => {
                flags(args, 7)?;
                return Ok(Request::Difference(decode(args)?));
            }
            id if id == tl::functions::updates::GetChannelDifference::CONSTRUCTOR_ID => {
                flags(args, 1)?;
                return Ok(Request::Channel(decode(args)?));
            }
            id if id == tl::functions::messages::ReceivedQueue::CONSTRUCTOR_ID => {
                return Ok(Request::Queue(
                    decode::<tl::functions::messages::ReceivedQueue>(args)?.max_qts,
                ));
            }
            id if id == tl::functions::updates::GetState::CONSTRUCTOR_ID => {
                return Ok(Request::State);
            }
            id if id == tl::functions::Ping::CONSTRUCTOR_ID => {
                return Ok(Request::Ping(decode::<tl::functions::Ping>(args)?.ping_id));
            }
            _ => {
                let name = tl::name_for_id(id);
                // A raw packed/container message could contain an inspected RPC
                // inside a second MTProto envelope. Future generic wrappers must
                // receive an explicit parser before this pinned hook accepts them.
                if name == "(unknown)"
                    || name.starts_with("invoke")
                    || matches!(
                        name,
                        "gzip_packed" | "msg_container" | "msg_copy" | "rpc_result"
                    )
                {
                    return Err(Error::IngressUnavailable);
                }
                return Ok(Request::Other);
            }
        };
    }
    Err(Error::IngressUnavailable)
}
