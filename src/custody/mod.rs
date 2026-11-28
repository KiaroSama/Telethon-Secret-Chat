//! Finite durable and volatile custody. Exhaustion refuses participation; it
//! never silently evicts a required message and then claims protocol continuity.
pub mod bounded;
pub mod limits;
mod store_core;
pub mod transient;

use crate::{
    chat::Chat,
    error::{Error, Result},
};
pub use bounded::{CustodyStorage, PauseReason, PauseStatus};
pub use limits::{CustodyLimits, IngressLimits, TransientLimits};
use std::collections::BTreeSet;
pub use store_core::*;
pub use transient::{ProtectedFileStorage, TransientStorage};

pub(crate) fn now() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::InvalidState)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::InvalidState)
}

pub(crate) fn identities(chat: &Chat) -> BTreeSet<String> {
    chat.outbox
        .keys()
        .map(|seq| format!("out:{seq}"))
        .chain(
            chat.gaps
                .keys()
                .chain(chat.deliveries.keys())
                .chain(chat.service_deliveries.keys())
                .map(|seq| format!("in:{seq}")),
        )
        .collect()
}

pub(crate) fn usage(chat: &Chat) -> Result<(usize, usize)> {
    let count =
        chat.outbox.len() + chat.gaps.len() + chat.deliveries.len() + chat.service_deliveries.len();
    let bytes = serde_json::to_vec(chat)
        .map_err(|_| Error::InvalidState)?
        .len();
    Ok((count, bytes))
}

#[derive(Clone)]
pub struct RelayDelivery {
    pub chat_id: i32,
    pub seq_no: i32,
    pub message: crate::chat::Envelope,
}
