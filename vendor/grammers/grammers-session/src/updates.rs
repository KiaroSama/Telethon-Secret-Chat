// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Logic and types for [Working with Updates](https://core.telegram.org/api/updates).

pub use crate::message_box::durable;
use grammers_tl_types::{self as tl, Deserializable as _};

pub use crate::message_box::{
    CapturedUpdate, Gap, MessageBox, MessageBoxes, PrematureEndReason, State, UpdateAndPeers,
    UpdatesLike,
};

impl UpdatesLike {
    /// Reconstruct request-dependent updates without discarding their original
    /// wire body. Malformed bodies remain exact receipts and trigger a gap.
    pub fn captured(envelope: Vec<u8>, request: Option<Vec<u8>>) -> Self {
        let update = match Self::from_bytes(&envelope) {
            Ok(Self::Updates(tl::enums::Updates::UpdateShortSentMessage(update))) => match request
                .as_ref()
                .and_then(|body| tl::functions::messages::SendMessage::from_boxed_bytes(body).ok())
            {
                Some(request) => Self::ShortSentMessage { request, update },
                None => Self::MalformedUpdates,
            },
            Ok(Self::AffectedMessages(affected)) => match request.as_ref().and_then(|body| {
                tl::functions::channels::DeleteMessages::from_boxed_bytes(body).ok()
            }) {
                Some(request) => match request.channel {
                    tl::enums::InputChannel::Channel(channel) => Self::AffectedChannelMessages {
                        affected,
                        channel_id: channel.channel_id,
                        message_ids: request.id,
                    },
                    _ => Self::AffectedMessages(affected),
                },
                None => Self::AffectedMessages(affected),
            },
            Ok(update) => update,
            Err(_) => Self::MalformedUpdates,
        };
        Self::Captured(CapturedUpdate {
            envelope,
            request,
            update: Box::new(update),
        })
    }
    /// Attempt to deserialize a buffer that may
    /// contain data pertaining to update handling.
    pub fn from_bytes(buf: &[u8]) -> tl::deserialize::Result<UpdatesLike> {
        match tl::enums::Updates::from_bytes(buf) {
            Ok(u) => Ok(UpdatesLike::Updates(u)),
            Err(e) => {
                if let Ok(tl::enums::messages::AffectedMessages::Messages(u)) =
                    tl::enums::messages::AffectedMessages::from_bytes(buf)
                {
                    Ok(UpdatesLike::AffectedMessages(u))
                } else if let Ok(tl::enums::messages::InvitedUsers::Users(u)) =
                    tl::enums::messages::InvitedUsers::from_bytes(buf)
                {
                    Ok(UpdatesLike::InvitedUsers(u))
                } else if let Ok(tl::enums::messages::ChatInviteJoinResult::Ok(u)) =
                    tl::enums::messages::ChatInviteJoinResult::from_bytes(buf)
                {
                    Ok(UpdatesLike::ChatInviteJoinResult(u))
                } else {
                    Err(e)
                }
            }
        }
    }
}
