use super::{Refused, TestResult, account::Manager};
use crate::support_live::Recorder;
use kiaro_secret_chat::{
    grammers_mtsender::UpdatesReceiver, grammers_session::types::UpdateState,
    grammers_tl_types as tl, manager::Event,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinHandle};
pub struct Pump {
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<UpdatesReceiver>>,
}
pub(super) fn belongs(
    update: &tl::enums::Update,
    user: i64,
    peer: i64,
    scope: &Arc<Mutex<BTreeSet<i32>>>,
) -> bool {
    let mut scope = match scope.lock() {
        Ok(scope) => scope,
        Err(_) => return false,
    };
    match update {
        tl::enums::Update::Encryption(update) => {
            let pair = match &update.chat {
                tl::enums::EncryptedChat::Requested(c) => Some((c.admin_id, c.participant_id)),
                tl::enums::EncryptedChat::Waiting(c) => Some((c.admin_id, c.participant_id)),
                tl::enums::EncryptedChat::Chat(c) => Some((c.admin_id, c.participant_id)),
                _ => None,
            };
            if pair.is_some_and(|(a, b)| (a == user && b == peer) || (a == peer && b == user)) {
                scope.insert(update.chat.id());
            }
            scope.contains(&update.chat.id())
        }
        tl::enums::Update::NewEncryptedMessage(update) => scope.contains(&match &update.message {
            tl::enums::EncryptedMessage::Message(m) => m.chat_id,
            tl::enums::EncryptedMessage::Service(m) => m.chat_id,
        }),
        _ => false,
    }
}
impl Pump {
    /// Offline ownership probe: no receiver, client or authorization is opened.
    #[allow(dead_code, clippy::expect_used)]
    pub(crate) async fn verify_cancelled_pause_aborts_task() {
        struct NotifyDrop(Option<oneshot::Sender<()>>);
        impl Drop for NotifyDrop {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }
        let (started, ready) = oneshot::channel();
        let (finished, dropped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _drop = NotifyDrop(Some(finished));
            let _ = started.send(());
            std::future::pending::<UpdatesReceiver>().await
        });
        ready.await.expect("the offline task must have started");
        let mut pause = Box::pin(
            Pump {
                stop: None,
                task: Some(task),
            }
            .pause(),
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(1), &mut pause)
                .await
                .is_err()
        );
        drop(pause);
        tokio::time::timeout(Duration::from_secs(1), dropped)
            .await
            .expect("cancelled pause must abort its owned task")
            .expect("the task's drop observer must run");
    }
    #[allow(clippy::too_many_arguments)] // All observers belong to this one caller-owned stream.
    pub fn spawn(
        mut receiver: UpdatesReceiver,
        manager: Manager,
        recorder: Recorder,
        fingerprints: Arc<Mutex<BTreeMap<i32, i64>>>,
        scope: Arc<Mutex<BTreeSet<i32>>>,
        user: i64,
        peer: i64,
        accept: bool,
    ) -> Self {
        let (stop, mut stopping) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                let next =
                    tokio::select! {biased;_=&mut stopping=>break,result=receiver.next()=>result};
                let output = match next {
                    Ok(output) => output,
                    Err(_) => {
                        recorder.record(Event::DecryptFailed {
                            chat_id: 0,
                            reason: "the live caller update stream failed",
                        });
                        break;
                    }
                };
                for (update, _) in output.0 {
                    if !belongs(&update, user, peer, &scope) {
                        continue;
                    }
                    if let tl::enums::Update::Encryption(encryption) = &update
                        && let tl::enums::EncryptedChat::Chat(chat) = &encryption.chat
                        && let Ok(mut known) = fingerprints.lock()
                    {
                        known.insert(chat.id, chat.key_fingerprint);
                    }
                    let request = if accept {
                        match &update {
                            tl::enums::Update::Encryption(encryption) => match &encryption.chat {
                                tl::enums::EncryptedChat::Requested(chat) => Some(chat.id),
                                _ => None,
                            },
                            _ => None,
                        }
                    } else {
                        None
                    };
                    manager.handle_update(&update).await;
                    if let Some(id) = request
                        && manager.accept(id).await.is_err()
                    {
                        recorder.record(Event::DecryptFailed {
                            chat_id: id,
                            reason: "the expected live peer request could not be accepted",
                        });
                    }
                }
            }
            receiver
        });
        Self {
            stop: Some(stop),
            task: Some(task),
        }
    }
    pub async fn pause(mut self) -> TestResult<UpdatesReceiver> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let task = self
            .task
            .as_mut()
            .ok_or(Refused("The live update-pump task is unavailable"))?;
        // Keep the abort owner in self even if an outer cleanup timeout drops
        // this future while the update handler or task join is still pending.
        match tokio::time::timeout(Duration::from_secs(15), &mut *task).await {
            Ok(Ok(receiver)) => Ok(receiver),
            Ok(Err(_)) => Err(Refused("The live caller update task failed").into()),
            Err(_) => {
                task.abort();
                let _ = task.await;
                Err(Refused(
                    "The live caller update pump could not pause within its cleanup deadline",
                )
                .into())
            }
        }
    }
}
impl Drop for Pump {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl super::Account {
    /// A caller-issued synchronization RPC on the same original Client.
    pub async fn force_difference(&self) -> TestResult {
        let state = self
            .handle
            .session
            .updates_state()
            .await
            .map_err(|_| Refused("The caller update state is unavailable"))?;
        let mut request = tl::functions::updates::GetDifference {
            pts: state.pts,
            pts_limit: None,
            pts_total_limit: None,
            date: state.date,
            qts: state.qts,
            qts_limit: None,
        };
        for _ in 0..32 {
            let response = self
                .client
                .invoke(&request)
                .await
                .map_err(|_| Refused("The caller difference synchronization request failed"))?;
            let (messages, updates, state, finished) =
                match response {
                    tl::enums::updates::Difference::Empty(_) => return Ok(()),
                    tl::enums::updates::Difference::TooLong(_) => return Err(Refused(
                        "The live account requires a complete ordinary update resynchronization",
                    )
                    .into()),
                    tl::enums::updates::Difference::Difference(d) => {
                        (d.new_encrypted_messages, d.other_updates, d.state, true)
                    }
                    tl::enums::updates::Difference::Slice(d) => (
                        d.new_encrypted_messages,
                        d.other_updates,
                        d.intermediate_state,
                        false,
                    ),
                };
            for message in messages {
                let update: tl::enums::Update =
                    tl::types::UpdateNewEncryptedMessage { message, qts: 0 }.into();
                if belongs(
                    &update,
                    self.config.user_id,
                    self.config.peer_user_id,
                    &self.scope,
                ) {
                    self.manager.handle_update(&update).await;
                }
            }
            for update in updates {
                if belongs(
                    &update,
                    self.config.user_id,
                    self.config.peer_user_id,
                    &self.scope,
                ) {
                    self.manager.handle_update(&update).await;
                }
            }
            let tl::enums::updates::State::State(state) = state;
            self.handle
                .session
                .set_update_state(UpdateState::Primary {
                    pts: state.pts,
                    date: state.date,
                    seq: state.seq,
                })
                .await
                .map_err(|_| Refused("The caller synchronization state could not be saved"))?;
            self.handle
                .session
                .set_update_state(UpdateState::Secondary { qts: state.qts })
                .await
                .map_err(|_| Refused("The caller synchronization frontier could not be saved"))?;
            if finished {
                return Ok(());
            }
            request.pts = state.pts;
            request.qts = state.qts;
            request.date = state.date;
        }
        Err(Refused("The live difference exceeded the bounded slice count").into())
    }
}
