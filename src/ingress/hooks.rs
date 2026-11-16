use super::{
    coordinator::DurableIngress,
    requests::{self, Request},
};
use grammers_mtsender::{DurableIngressHook, IngressReceiptKind, IngressRefused};
use grammers_session::updates::{MessageBoxes, UpdateAndPeers};
use grammers_tl_types as tl;
use sha2::Digest;
use std::sync::Arc;
impl DurableIngressHook for DurableIngress {
    fn prepare_attachment(&self) -> std::result::Result<(), IngressRefused> {
        let mut core = self.core.lock().map_err(|_| IngressRefused)?;
        if core.attached || core.stopped || core.released || core.failed {
            return Err(IngressRefused);
        }
        let ledger = core.attachment.take().ok_or(IngressRefused)?;
        let state = core.state.clone();
        core.persist(state, ledger).map_err(|_| IngressRefused)?;
        core.attached = true;
        Ok(())
    }
    fn stage_live(
        &self,
        dc_id: i32,
        input: &grammers_session::updates::UpdatesLike,
    ) -> std::result::Result<(), IngressRefused> {
        let mut core = self.core.lock().map_err(|_| IngressRefused)?;
        core.ready().map_err(|_| IngressRefused)?;
        if dc_id != core.backend.binding().dc_id() {
            core.failed = true;
            return Err(IngressRefused);
        }
        let mut ledger = core.ledger.clone();
        let state = core.state.clone();
        let result = grammers_session::updates::durable::encode_live(input)
            .map_err(|_| crate::error::Error::Capacity)
            .and_then(|envelope| ledger.stage_live(envelope, &core.limits))
            .and_then(|_| core.persist(state, ledger));
        if result.is_err() {
            core.failed = true;
        }
        self.notification.notify_one();
        result.map_err(|_| IngressRefused)
    }
    fn pending_live(
        &self,
    ) -> std::result::Result<Vec<grammers_session::updates::UpdatesLike>, IngressRefused> {
        let core = self.core.lock().map_err(|_| IngressRefused)?;
        core.ready().map_err(|_| IngressRefused)?;
        core.ledger.pending_live().map_err(|_| IngressRefused)
    }
    fn receipt_notification(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.notification)
    }
    fn restore(&self) -> std::result::Result<Vec<u8>, IngressRefused> {
        self.checkpoint().map_err(|_| IngressRefused)
    }
    fn replay(&self) -> std::result::Result<Vec<(u64, UpdateAndPeers)>, IngressRefused> {
        let core = self.core.lock().map_err(|_| IngressRefused)?;
        core.ready().map_err(|_| IngressRefused)?;
        core.ledger
            .batches
            .iter()
            .map(|batch| {
                let mut output = batch.updates().map_err(|_| IngressRefused)?;
                let mut index = 0;
                output.0.retain(|_| {
                    let retain = !batch.applied.contains(&index);
                    index += 1;
                    retain
                });
                Ok((batch.id, output))
            })
            .collect()
    }
    fn commit(
        &self,
        before: &[u8],
        after: &[u8],
        kind: IngressReceiptKind,
        envelope: &[u8],
        output: &UpdateAndPeers,
    ) -> std::result::Result<Option<u64>, IngressRefused> {
        let mut core = self.core.lock().map_err(|_| IngressRefused)?;
        let result = core.capture(before, after, kind, envelope, &[], output);
        if result.is_ok() && kind != IngressReceiptKind::Machine {
            self.notification.notify_one();
        }
        if result.is_err() {
            core.failed = true;
        }
        result.map_err(|_| IngressRefused)
    }
    fn guard_request(&self, dc_id: i32, body: &[u8]) -> std::result::Result<u64, IngressRefused> {
        let mut core = self.core.lock().map_err(|_| IngressRefused)?;
        core.ready().map_err(|_| IngressRefused)?;
        let request = requests::inspect(body).map_err(|_| IngressRefused)?;
        if let Request::Ping(nonce) = request
            && core.proof.is_some_and(|proof| proof.0 == nonce)
        {
            core.proof = Some((nonce, true));
        }
        let frontier = match request {
            Request::Difference(ref request) => Some(request.qts),
            Request::Queue(qts) => Some(qts),
            Request::State => return Err(IngressRefused),
            _ => None,
        };
        if let Some(qts) = frontier
            && (!core.verified
                || dc_id != core.backend.binding().dc_id()
                || qts < 0
                || qts > core.ledger.frontier().map_err(|_| IngressRefused)?)
        {
            return Err(IngressRefused);
        }
        if core.issued.len() >= 64 {
            return Err(IngressRefused);
        }
        let permit = core.next_permit;
        core.next_permit = permit.checked_add(1).ok_or(IngressRefused)?;
        core.issued
            .insert(permit, (dc_id, sha2::Sha256::digest(body).into()));
        Ok(permit)
    }
    fn guard_result(
        &self,
        permit: u64,
        dc_id: i32,
        request: &[u8],
        response: &[u8],
    ) -> std::result::Result<(), IngressRefused> {
        let mut core = self.core.lock().map_err(|_| IngressRefused)?;
        core.ready().map_err(|_| IngressRefused)?;
        if core.issued.remove(&permit) != Some((dc_id, sha2::Sha256::digest(request).into())) {
            return Err(IngressRefused);
        }
        let request_kind = requests::inspect(request).map_err(|_| IngressRefused)?;
        let before = core
            .ledger
            .checkpoint()
            .map_err(|_| IngressRefused)?
            .to_vec();
        let mut machine =
            MessageBoxes::from_durable_checkpoint(&before).map_err(|_| IngressRefused)?;
        let (kind, output) = match request_kind {
            Request::Difference(request) => {
                if request.qts != machine.session_state().qts {
                    return Err(IngressRefused);
                }
                let response = requests::decode::<tl::enums::updates::Difference>(response)
                    .map_err(|_| IngressRefused)?;
                if matches!(response, tl::enums::updates::Difference::TooLong(_)) {
                    core.failed = true;
                    return Err(IngressRefused);
                }
                machine
                    .begin_durable_difference(None)
                    .map_err(|_| IngressRefused)?;
                (
                    IngressReceiptKind::Difference,
                    machine.apply_difference(response),
                )
            }
            Request::Channel(request) => {
                let tl::enums::InputChannel::Channel(channel) = request.channel else {
                    return Err(IngressRefused);
                };
                machine
                    .begin_durable_difference(Some((channel.channel_id, request.pts)))
                    .map_err(|_| IngressRefused)?;
                let response = requests::decode::<tl::enums::updates::ChannelDifference>(response)
                    .map_err(|_| IngressRefused)?;
                (
                    IngressReceiptKind::ChannelDifference,
                    machine.apply_channel_difference(response),
                )
            }
            _ => return Ok(()),
        };
        let after = machine.durable_checkpoint().map_err(|_| IngressRefused)?;
        let result = core.capture(&before, &after, kind, response, request, &output);
        if result.is_ok() {
            self.notification.notify_one();
        }
        if result.is_err() {
            core.failed = true;
        }
        result.map(|_| ()).map_err(|_| IngressRefused)
    }
    fn cancel_request(&self, permit: u64) {
        if let Ok(mut core) = self.core.lock() {
            core.issued.remove(&permit);
        }
    }
}
