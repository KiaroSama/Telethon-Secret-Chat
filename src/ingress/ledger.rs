//! Encrypted journal data. This module never implements Debug for held bodies.
use crate::{
    custody::IngressLimits,
    error::{Error, Result},
    storage::{SecretRecord, State},
};
use grammers_mtsender::IngressReceiptKind;
use grammers_session::updates::{MessageBoxes, UpdateAndPeers, durable};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub(super) const RECORD: &str = "ingress/journal-v1";
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    Live,
    Difference,
    ChannelDifference,
}
impl TryFrom<IngressReceiptKind> for ReceiptKind {
    type Error = Error;
    fn try_from(kind: IngressReceiptKind) -> Result<Self> {
        match kind {
            IngressReceiptKind::Live => Ok(Self::Live),
            IngressReceiptKind::Difference => Ok(Self::Difference),
            IngressReceiptKind::ChannelDifference => Ok(Self::ChannelDifference),
            _ => Err(Error::InvalidState),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressBatch {
    pub id: u64,
    pub identity: String,
    pub kind: ReceiptKind,
    pub envelope: SecretRecord,
    pub request: SecretRecord,
    pub before: SecretRecord,
    pub after: SecretRecord,
    pub output: SecretRecord,
    pub cursor: usize,
    pub applied: BTreeSet<usize>,
    pub created_ms: u64,
}
impl IngressBatch {
    pub fn updates(&self) -> Result<UpdateAndPeers> {
        durable::decode_output(self.output.as_bytes()).map_err(|_| Error::InvalidState)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputReceipt {
    id: u64,
    envelope: SecretRecord,
    created_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    version: u32,
    pub checkpoint: Option<SecretRecord>,
    #[serde(default)]
    receipt_frontier: Option<i32>,
    next_id: u64,
    pub batches: Vec<IngressBatch>,
    #[serde(default)]
    inputs: Vec<InputReceipt>,
    pub paused: bool,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: 1,
            checkpoint: None,
            receipt_frontier: None,
            next_id: 1,
            batches: vec![],
            inputs: vec![],
            paused: false,
        }
    }
}
pub(super) fn now() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .ok_or(Error::Storage)
}
impl Ledger {
    pub fn stage_live(&mut self, envelope: Vec<u8>, limits: &IngressLimits) -> Result<()> {
        if self.paused || envelope.len() > limits.batch_bytes {
            return Err(Error::Capacity);
        }
        if self
            .inputs
            .iter()
            .any(|row| row.envelope.as_bytes() == envelope)
        {
            return Ok(());
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(Error::Capacity)?;
        self.inputs.push(InputReceipt {
            id,
            envelope: SecretRecord::new(envelope),
            created_ms: now()?,
        });
        self.check_budget(limits)
    }
    pub fn pending_live(&self) -> Result<Vec<grammers_session::updates::UpdatesLike>> {
        self.inputs
            .iter()
            .map(|row| {
                durable::decode_live(row.envelope.as_bytes()).map_err(|_| Error::InvalidState)
            })
            .collect()
    }
    pub fn load(state: &State, limits: &IngressLimits) -> Result<Self> {
        let ledger = match state.private_records.get(RECORD) {
            Some(row) => serde_json::from_slice(row.as_bytes()).map_err(|_| Error::InvalidState)?,
            None if state.chats.is_empty() && state.private_records.is_empty() => Self::default(),
            None => return Err(Error::InvalidState),
        };
        ledger.validate(limits)?;
        Ok(ledger)
    }
    pub fn save(&self, state: &mut State) -> Result<()> {
        state.private_records.insert(
            RECORD.into(),
            SecretRecord::new(serde_json::to_vec(self).map_err(|_| Error::InvalidState)?),
        );
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<&[u8]> {
        self.checkpoint
            .as_ref()
            .map(SecretRecord::as_bytes)
            .ok_or(Error::IngressUnavailable)
    }
    pub fn frontier(&self) -> Result<i32> {
        let qts = MessageBoxes::from_durable_checkpoint(self.checkpoint()?)
            .map_err(|_| Error::InvalidState)?
            .session_state()
            .qts;
        if self.receipt_frontier != Some(qts) {
            return Err(Error::InvalidState);
        }
        Ok(qts)
    }
    pub fn validate(&self, limits: &IngressLimits) -> Result<()> {
        limits.validate()?;
        if self.version != 1
            || self.next_id == 0
            || self.batches.len() + self.inputs.len() > limits.records
        {
            return Err(Error::InvalidState);
        }
        if let Some(checkpoint) = &self.checkpoint {
            let qts = MessageBoxes::from_durable_checkpoint(checkpoint.as_bytes())
                .map_err(|_| Error::InvalidState)?
                .session_state()
                .qts;
            if self.receipt_frontier != Some(qts) {
                return Err(Error::InvalidState);
            }
        } else if !self.batches.is_empty()
            || !self.inputs.is_empty()
            || self.receipt_frontier.is_some()
        {
            return Err(Error::InvalidState);
        }
        let timestamp = now()?;
        let mut ids = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for input in &self.inputs {
            if input.id == 0
                || input.id >= self.next_id
                || !ids.insert(input.id)
                || input.created_ms == 0
                || input.created_ms > timestamp
                || input.envelope.as_bytes().len() > limits.batch_bytes
            {
                return Err(Error::InvalidState);
            }
            durable::decode_live(input.envelope.as_bytes()).map_err(|_| Error::InvalidState)?;
        }
        for batch in &self.batches {
            if batch.id == 0
                || batch.id >= self.next_id
                || !ids.insert(batch.id)
                || batch.identity.len() != 64
                || !batch.identity.bytes().all(|b| b.is_ascii_hexdigit())
                || !identities.insert(&batch.identity)
                || batch.created_ms == 0
                || batch.created_ms > timestamp
                || batch.envelope.as_bytes().len() > limits.batch_bytes
            {
                return Err(Error::InvalidState);
            }
            MessageBoxes::from_durable_checkpoint(batch.before.as_bytes())
                .map_err(|_| Error::InvalidState)?;
            MessageBoxes::from_durable_checkpoint(batch.after.as_bytes())
                .map_err(|_| Error::InvalidState)?;
            let count = batch.updates()?.0.len();
            if batch.cursor > count
                || batch.applied.iter().any(|index| *index >= count)
                || (0..batch.cursor).any(|i| !batch.applied.contains(&i))
                || batch.applied.contains(&batch.cursor)
            {
                return Err(Error::InvalidState);
            }
        }
        self.check_budget(limits)
    }
    pub fn check_budget(&self, limits: &IngressLimits) -> Result<()> {
        if self.batches.len() + self.inputs.len() > limits.records
            || serde_json::to_vec(self)
                .map_err(|_| Error::InvalidState)?
                .len()
                > limits.bytes
        {
            return Err(Error::Capacity);
        }
        let timestamp = now()?;
        if self.batches.iter().any(|batch| {
            timestamp.saturating_sub(batch.created_ms) as f64 >= limits.age_seconds * 1000.0
        }) || self.inputs.iter().any(|row| {
            timestamp.saturating_sub(row.created_ms) as f64 >= limits.age_seconds * 1000.0
        }) {
            return Err(Error::Paused);
        }
        Ok(())
    }
    pub fn bootstrap(&mut self, checkpoint: Vec<u8>) -> Result<()> {
        if self.checkpoint.is_some() || !self.batches.is_empty() || !self.inputs.is_empty() {
            return Err(Error::IngressUnavailable);
        }
        let qts = MessageBoxes::from_durable_checkpoint(&checkpoint)
            .map_err(|_| Error::InvalidState)?
            .session_state()
            .qts;
        self.receipt_frontier = Some(qts);
        self.checkpoint = Some(SecretRecord::new(checkpoint));
        Ok(())
    }
    pub fn capture(
        &mut self,
        (before, after): (&[u8], &[u8]),
        kind: IngressReceiptKind,
        envelope: &[u8],
        request: &[u8],
        output: &UpdateAndPeers,
        limits: &IngressLimits,
    ) -> Result<Option<u64>> {
        if self.paused || before != self.checkpoint()? {
            return Err(Error::Paused);
        }
        let next = MessageBoxes::from_durable_checkpoint(after).map_err(|_| Error::InvalidState)?;
        if next.session_state().qts < self.frontier()? {
            return Err(Error::Sequence);
        }
        if kind == IngressReceiptKind::Machine {
            if next.session_state().qts != self.frontier()?
                || !envelope.is_empty()
                || !output.0.is_empty()
            {
                return Err(Error::InvalidState);
            }
            self.checkpoint = Some(SecretRecord::new(after.to_vec()));
            self.check_budget(limits)?;
            return Ok(None);
        }
        if envelope.len() > limits.batch_bytes {
            return Err(Error::Capacity);
        }
        let kind = ReceiptKind::try_from(kind)?;
        let input = if kind == ReceiptKind::Live {
            self.inputs
                .iter()
                .position(|row| row.envelope.as_bytes() == envelope)
                .map(|index| self.inputs.remove(index))
        } else {
            None
        };
        let identity = identity(kind, envelope, request);
        if let Some(batch) = self.batches.iter().find(|batch| batch.identity == identity) {
            self.receipt_frontier = Some(next.session_state().qts);
            self.checkpoint = Some(SecretRecord::new(after.to_vec()));
            return Ok(Some(batch.id));
        }
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(Error::Capacity)?;
        self.batches.push(IngressBatch {
            id,
            identity,
            kind,
            envelope: SecretRecord::new(envelope.to_vec()),
            request: SecretRecord::new(request.to_vec()),
            before: SecretRecord::new(before.to_vec()),
            after: SecretRecord::new(after.to_vec()),
            output: SecretRecord::new(
                durable::encode_output(output).map_err(|_| Error::InvalidState)?,
            ),
            cursor: 0,
            applied: BTreeSet::new(),
            created_ms: input.map_or_else(now, |row| Ok(row.created_ms))?,
        });
        self.receipt_frontier = Some(next.session_state().qts);
        self.checkpoint = Some(SecretRecord::new(after.to_vec()));
        self.check_budget(limits)?;
        Ok(Some(id))
    }
    pub fn mark(&mut self, id: u64, index: usize, hash: &[u8; 32]) -> Result<()> {
        let batch = self
            .batches
            .iter_mut()
            .find(|batch| batch.id == id)
            .ok_or(Error::InvalidState)?;
        let output = batch.updates()?;
        let (update, _) = output.0.get(index).ok_or(Error::InvalidState)?;
        use grammers_tl_types::Serializable;
        if <[u8; 32]>::from(Sha256::digest(update.to_bytes())) != *hash {
            return Err(Error::InvalidState);
        }
        batch.applied.insert(index);
        while batch.applied.contains(&batch.cursor) {
            batch.cursor += 1;
        }
        Ok(())
    }
    pub fn prune_completed(&mut self) -> Result<()> {
        let mut remaining = Vec::new();
        for batch in self.batches.drain(..) {
            if batch.cursor < batch.updates()?.0.len() {
                remaining.push(batch);
            }
        }
        self.batches = remaining;
        Ok(())
    }
}
fn identity(kind: ReceiptKind, envelope: &[u8], request: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"kiaro-ingress-receipt-v1\0");
    hash.update([match kind {
        ReceiptKind::Live => 0,
        ReceiptKind::Difference => 1,
        ReceiptKind::ChannelDifference => 2,
    }]);
    hash.update((request.len() as u64).to_le_bytes());
    hash.update(request);
    hash.update(envelope);
    hex::encode(hash.finalize())
}
