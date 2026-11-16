// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use grammers_session::storages::ErasedSession;
use grammers_session::types::{PeerId, PeerInfo, UpdateState, UpdatesState};
use grammers_session::updates::{
    MessageBoxes, PrematureEndReason, State, UpdateAndPeers, UpdatesLike,
};
use grammers_tl_types::{self as tl, Deserializable, Serializable};
use tokio::sync::mpsc;
use tokio::time::timeout_at;
use tracing::{info, trace, warn};

use crate::durable_ingress::DurableWorker;
use crate::{IngressReceiptKind, IngressRefused};
use crate::{InvocationError, SenderPoolHandle, UpdatesConfiguration};

// See https://core.telegram.org/method/updates.getChannelDifference.
const BOT_CHANNEL_DIFF_LIMIT: i32 = 100000;
const USER_CHANNEL_DIFF_LIMIT: i32 = 100;

pub struct UpdatesReceiver {
    durable: Option<DurableWorker>,
    seen_receipts: BTreeSet<u64>,
    handle: SenderPoolHandle,
    session: Arc<ErasedSession>,
    message_box: MessageBoxes,
    updates: mpsc::Receiver<UpdatesLike>,
    buffer: VecDeque<UpdateAndPeers>,
    should_get_state: bool,
}

async fn prepare_channel_difference(
    mut request: tl::functions::updates::GetChannelDifference,
    session: Arc<ErasedSession>,
) -> Result<
    Option<tl::functions::updates::GetChannelDifference>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let id = match &request.channel {
        tl::enums::InputChannel::Channel(channel) => PeerId::channel_unchecked(channel.channel_id),
        _ => unreachable!(),
    };

    if let Some(PeerInfo::Channel {
        id,
        auth: Some(auth),
        ..
    }) = session.peer(id).await?
    {
        request.channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: id,
            access_hash: auth.hash(),
        });
        request.limit = if session
            .peer(PeerId::self_user())
            .await?
            .map(|user| match user {
                PeerInfo::User { bot, .. } => bot.unwrap_or(false),
                _ => false,
            })
            .unwrap_or(false)
        {
            BOT_CHANNEL_DIFF_LIMIT
        } else {
            USER_CHANNEL_DIFF_LIMIT
        };
        trace!("requesting channel difference");
        Ok(Some(request))
    } else {
        warn!(
            channel_id = ?id,
            "cannot getChannelDifference as we're missing its hash",
        );
        Ok(None)
    }
}

fn requested_channel_id(channel: &tl::enums::InputChannel) -> Result<i64, IngressRefused> {
    match channel {
        tl::enums::InputChannel::Channel(channel) => Ok(channel.channel_id),
        tl::enums::InputChannel::FromMessage(channel) => Ok(channel.channel_id),
        _ => Err(IngressRefused),
    }
}

impl UpdatesReceiver {
    pub async fn create(
        handle: SenderPoolHandle,
        session: Arc<ErasedSession>,
        updates: mpsc::Receiver<UpdatesLike>,
        configuration: UpdatesConfiguration,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let durable = handle.durable_worker()?;
        let (checkpoint, replay) = match &durable {
            Some(worker) => {
                let (checkpoint, replay) = worker
                    .run(|hook| Ok((hook.restore()?, hook.replay()?)))
                    .await?;
                (Some(checkpoint), replay)
            }
            None => (None, Vec::new()),
        };
        let mut seen_receipts = BTreeSet::new();
        let buffer = replay
            .into_iter()
            .map(|(id, output)| {
                seen_receipts.insert(id);
                output
            })
            .collect();
        let message_box = if let Some(checkpoint) = &checkpoint {
            MessageBoxes::from_durable_checkpoint(checkpoint)?
        } else if configuration.catch_up {
            MessageBoxes::load(session.updates_state().await?)
        } else {
            // If the user doesn't want to bother with catching up on previous update, start with
            // pristine state instead.
            MessageBoxes::new()
        };
        // Don't bother getting pristine update state if we're not logged in.
        let should_get_state = durable.is_none()
            && message_box.is_empty()
            && session.peer(PeerId::self_user()).await?.is_some();
        handle.claim_durable_receiver(durable.is_some())?;

        Ok(UpdatesReceiver {
            durable,
            seen_receipts,
            handle,
            session,
            message_box,
            updates,
            buffer,
            should_get_state,
        })
    }

    /// Pops an updates from the queue, waiting for an updates to arrive first if the queue is empty.
    pub async fn next(
        &mut self,
    ) -> Result<
        (
            Vec<(tl::enums::Update, State)>,
            Vec<tl::enums::User>,
            Vec<tl::enums::Chat>,
        ),
        InvocationError,
    > {
        let session = self.session.clone();

        if self.should_get_state {
            self.should_get_state = false;
            match self.invoke(&tl::functions::updates::GetState {}).await {
                Ok(tl::enums::updates::State::State(state)) => {
                    session
                        .clone()
                        .set_update_state(UpdateState::All(UpdatesState {
                            pts: state.pts,
                            qts: state.qts,
                            date: state.date,
                            seq: state.seq,
                            channels: Vec::new(),
                        }))
                        .await?;
                    self.message_box.set_state(state);
                }
                Err(_err) => {
                    // The account may no longer actually be logged in, or it can rarely fail.
                    // `message_box` will try to correct its state as updates arrive.
                }
            }
        }

        loop {
            if self.buffer.is_empty() {
                if let Some(hook) = &self.durable {
                    if let Some(input) = hook
                        .run(|hook| hook.pending_live())
                        .await?
                        .into_iter()
                        .next()
                    {
                        self.process_staged_updates(input).await?;
                        continue;
                    }
                }
            }
            let (deadline, get_diff, get_channel_diff) = {
                if let Some(update) = self.buffer.pop_front() {
                    return Ok(update);
                }
                (
                    self.check_deadlines().await?, // persist difference ownership before using its request
                    self.message_box.get_difference(),
                    match self.message_box.get_channel_difference() {
                        Some(gd) => {
                            let channel_id = requested_channel_id(&gd.channel)?;
                            match prepare_channel_difference(gd, session.clone()).await? {
                                Some(request) => Some(request),
                                None => {
                                    self.end_channel_difference(
                                        channel_id,
                                        PrematureEndReason::Banned,
                                    )
                                    .await?;
                                    None
                                }
                            }
                        }
                        None => None,
                    },
                )
            };
            if !self.buffer.is_empty() {
                continue;
            }

            if let Some(request) = get_diff {
                let response = self.invoke(&request).await?;
                if self.durable.is_some() {
                    self.refresh_durable().await?;
                    continue;
                }
                let envelope = response.to_bytes();
                self.transition(IngressReceiptKind::Difference, envelope, move |machine| {
                    ((), Some(machine.apply_difference(response)))
                })
                .await?;
                continue;
            }

            if let Some(request) = get_channel_diff {
                let channel_id = requested_channel_id(&request.channel)?;
                let maybe_response = self.invoke(&request).await;

                let response = match maybe_response {
                    Ok(r) => r,
                    Err(e) if e.is("PERSISTENT_TIMESTAMP_OUTDATED") => {
                        // According to Telegram's docs:
                        // "Channel internal replication issues, try again later (treat this like an RPC_CALL_FAIL)."
                        // We can treat this as "empty difference" and not update the local pts.
                        // Then this same call will be retried when another gap is detected or timeout expires.
                        //
                        // Another option would be to literally treat this like an RPC_CALL_FAIL and retry after a few
                        // seconds, but if Telegram is having issues it's probably best to wait for it to send another
                        // update (hinting it may be okay now) and retry then.
                        //
                        // This is a bit hacky because MessageBox doesn't really have a way to "not update" the pts.
                        // Instead we manually extract the previously-known pts and use that.
                        warn!(
                            "Getting difference for channel updates caused PersistentTimestampOutdated; ending getting difference prematurely until server issues are resolved"
                        );
                        {
                            self.end_channel_difference(
                                channel_id,
                                PrematureEndReason::TemporaryServerIssues,
                            )
                            .await?;
                        }
                        continue;
                    }
                    Err(e) if e.is("CHANNEL_PRIVATE") => {
                        info!("Account is now banned from channel difference");
                        {
                            self.end_channel_difference(channel_id, PrematureEndReason::Banned)
                                .await?;
                        }
                        continue;
                    }
                    Err(InvocationError::Rpc(rpc_error)) if rpc_error.code == 500 => {
                        warn!("Telegram reported an internal channel-difference error");
                        {
                            self.end_channel_difference(
                                channel_id,
                                PrematureEndReason::TemporaryServerIssues,
                            )
                            .await?;
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                if self.durable.is_some() {
                    self.refresh_durable().await?;
                    continue;
                }

                let envelope = response.to_bytes();
                self.transition(
                    IngressReceiptKind::ChannelDifference,
                    envelope,
                    move |machine| ((), Some(machine.apply_channel_difference(response))),
                )
                .await?;
                continue;
            }

            let received = if let Some(hook) = &self.durable {
                let notification = hook.hook.receipt_notification();
                tokio::select! {
                    biased;
                    _=notification.notified()=>{self.refresh_durable().await?;continue;},
                    received=timeout_at(deadline.into(),self.updates.recv())=>received,
                }
            } else {
                timeout_at(deadline.into(), self.updates.recv()).await
            };
            match received {
                Ok(Some(updates)) => self.process_socket_updates(updates).await?,
                Ok(None) => break Err(InvocationError::Dropped("updates channel was closed")),
                Err(_) => {}
            }
        }
    }

    /// Synchronize the updates state to the session.
    ///
    /// This is **not** automatically done on drop.
    pub async fn sync_update_state(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let state = if let Some(worker) = &self.durable {
            worker
                .run(|hook| {
                    let checkpoint = hook.restore()?;
                    Ok(MessageBoxes::from_durable_checkpoint(&checkpoint)
                        .map_err(|_| IngressRefused)?
                        .session_state())
                })
                .await?
        } else {
            if !self.buffer.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "cannot synchronize while received updates remain buffered",
                )
                .into());
            }
            if self.message_box.is_empty() {
                return Ok(());
            }
            self.message_box.session_state()
        };
        self.session
            .clone()
            .set_update_state(UpdateState::All(state))
            .await?;
        Ok(())
    }

    async fn invoke<R: tl::RemoteCall>(&self, request: &R) -> Result<R::Return, InvocationError> {
        let dc_id = self.session.clone().home_dc_id()?;
        self.handle
            .do_invoke_in_dc(dc_id, request.to_bytes())
            .await
            .and_then(|body| R::Return::from_bytes(&body).map_err(|e| e.into()))
    }

    async fn process_socket_updates(
        &mut self,
        updates: UpdatesLike,
    ) -> Result<(), InvocationError> {
        if let Some(worker) = &self.durable {
            let dc_id = self.session.home_dc_id()?;
            worker
                .run(move |hook| hook.stage_live(dc_id, &updates))
                .await?;
            return Ok(());
        }
        self.process_staged_updates(updates).await
    }
    async fn process_staged_updates(
        &mut self,
        updates: UpdatesLike,
    ) -> Result<(), InvocationError> {
        let envelope = if self.durable.is_some() {
            grammers_session::updates::durable::encode_live(&updates).map_err(|_| IngressRefused)?
        } else {
            Vec::new()
        };
        self.transition(IngressReceiptKind::Live, envelope, move |machine| {
            ((), machine.process_updates(updates).ok())
        })
        .await
    }
    async fn check_deadlines(&mut self) -> Result<std::time::Instant, InvocationError> {
        self.transition(IngressReceiptKind::Machine, Vec::new(), |machine| {
            (machine.check_deadlines(), None)
        })
        .await
    }
    async fn end_channel_difference(
        &mut self,
        channel_id: i64,
        reason: PrematureEndReason,
    ) -> Result<(), InvocationError> {
        self.transition(IngressReceiptKind::Machine, Vec::new(), move |machine| {
            machine.end_channel_difference_for(channel_id, reason);
            ((), None)
        })
        .await
    }
    async fn transition<T: Send + 'static>(
        &mut self,
        kind: IngressReceiptKind,
        envelope: Vec<u8>,
        transform: impl FnOnce(&mut MessageBoxes) -> (T, Option<UpdateAndPeers>) + Send + 'static,
    ) -> Result<T, InvocationError> {
        self.refresh_durable().await?;
        let (result, output) = if let Some(worker) = &self.durable {
            // One worker permit covers the entire read/apply/commit operation.
            // Caller-issued differences cannot replace its checkpoint midway.
            let (result, staged, receipt, output) = worker
                .run(move |hook| {
                    let before = hook.restore()?;
                    let mut staged = MessageBoxes::from_durable_checkpoint(&before)
                        .map_err(|_| IngressRefused)?;
                    let (result, output) = transform(&mut staged);
                    let after = staged.durable_checkpoint().map_err(|_| IngressRefused)?;
                    let receipt = hook.commit(
                        &before,
                        &after,
                        kind,
                        &envelope,
                        output
                            .as_ref()
                            .unwrap_or(&(Vec::new(), Vec::new(), Vec::new())),
                    )?;
                    Ok((result, staged, receipt, output))
                })
                .await?;
            if let Some(id) = receipt {
                self.seen_receipts.insert(id);
            }
            self.message_box = staged;
            (result, output)
        } else {
            transform(&mut self.message_box)
        };
        if let Some((updates, users, chats)) = output {
            self.extend_update_queue(updates, users, chats);
        }
        Ok(result)
    }
    async fn refresh_durable(&mut self) -> Result<(), InvocationError> {
        if let Some(worker) = &self.durable {
            let (checkpoint, pending) = worker
                .run(|hook| Ok((hook.restore()?, hook.replay()?)))
                .await?;
            let staged =
                MessageBoxes::from_durable_checkpoint(&checkpoint).map_err(|_| IngressRefused)?;
            let held = pending.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>();
            self.seen_receipts.retain(|id| held.contains(id));
            for (id, output) in pending {
                if self.seen_receipts.insert(id) {
                    self.buffer.push_back(output);
                }
            }
            self.message_box = staged;
        }
        Ok(())
    }

    fn extend_update_queue(
        &mut self,
        updates: Vec<(tl::enums::Update, State)>,
        users: Vec<tl::enums::User>,
        chats: Vec<tl::enums::Chat>,
    ) {
        self.buffer.push_back((updates, users, chats));
    }
}
impl Drop for UpdatesReceiver {
    fn drop(&mut self) {
        self.handle.release_durable_receiver();
    }
}
