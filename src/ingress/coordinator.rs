use super::{
    EncryptedIngressStorage, application,
    ledger::{IngressBatch, Ledger},
};
use crate::{
    binding::AuthorizationBinding,
    crypto::random_bytes,
    custody::IngressLimits,
    error::{Error, Result},
    storage::{State, Storage},
};
use grammers_mtsender::{DurableIngressHook, IngressReceiptKind, SenderPoolFatHandle};
use grammers_session::{
    storages::ErasedSession,
    types::{PeerId, PeerInfo},
    updates::UpdateAndPeers,
};
use grammers_tl_types::{self as tl, Serializable};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub(super) struct Core {
    pub backend: EncryptedIngressStorage,
    pub state: State,
    pub ledger: Ledger,
    pub limits: IngressLimits,
    pub attached: bool,
    pub(super) attachment: Option<Ledger>,
    pub verified: bool,
    pub stopped: bool,
    pub released: bool,
    pub failed: bool,
    pub session: Option<Arc<ErasedSession>>,
    pub proof: Option<(i64, bool)>,
    pub storage_claimed: bool,
    pub(super) issued: BTreeMap<u64, (i32, [u8; 32])>,
    pub(super) next_permit: u64,
}
impl Core {
    pub fn ready(&self) -> Result<()> {
        if !self.attached || self.stopped || self.released || self.failed {
            return Err(Error::IngressUnavailable);
        }
        self.check_session()?;
        self.ledger.check_size(&self.limits)
    }
    pub fn check_session(&self) -> Result<()> {
        let session = self.session.as_ref().ok_or(Error::IngressUnavailable)?;
        let binding = self.backend.binding();
        if session.home_dc_id().map_err(|_| Error::Authorization)? != binding.dc_id() {
            return Err(Error::Authorization);
        }
        let mut dc = session
            .dc_option(binding.dc_id())
            .map_err(|_| Error::Authorization)?
            .ok_or(Error::Authorization)?;
        let key = Zeroizing::new(dc.auth_key.take().ok_or(Error::Authorization)?);
        let digest = Sha1::digest(key.as_slice());
        let id = i64::from_le_bytes(
            digest[12..20]
                .try_into()
                .map_err(|_| Error::Authorization)?,
        );
        if binding.auth_key_id() != Some(id) {
            return Err(Error::Authorization);
        }
        Ok(())
    }
    pub fn persist(&mut self, mut state: State, ledger: Ledger) -> Result<()> {
        ledger.validate(&self.limits)?;
        ledger.save(&mut state)?;
        if let Err(error) = self.backend.commit(&state) {
            self.failed = true;
            return Err(error);
        }
        self.state = state;
        self.ledger = ledger;
        Ok(())
    }
    pub(super) fn capture(
        &mut self,
        before: &[u8],
        after: &[u8],
        kind: IngressReceiptKind,
        envelope: &[u8],
        request: &[u8],
        output: &UpdateAndPeers,
    ) -> Result<Option<u64>> {
        self.ready()?;
        let mut ledger = self.ledger.clone();
        let id = ledger.capture(
            (before, after),
            kind,
            envelope,
            request,
            output,
            &self.limits,
        )?;
        self.persist(self.state.clone(), ledger)?;
        Ok(id)
    }
}
/// Owns the encrypted receipt journal and its single secret-protocol writer.
/// Prepare on the original caller-owned pool before starting its runner.
pub struct DurableIngress {
    pub(super) core: Mutex<Core>,
    pub(super) owner: u64,
    prepare_lock: tokio::sync::Mutex<()>,
    pub(super) notification: Arc<tokio::sync::Notify>,
}
impl DurableIngress {
    pub fn new(backend: EncryptedIngressStorage, limits: IngressLimits) -> Result<Arc<Self>> {
        limits.validate()?;
        if limits.bytes > backend.max_bytes() {
            return Err(Error::Capacity);
        }
        let state = backend.load()?.ok_or(Error::InvalidState)?;
        let ledger = Ledger::load(&state, &limits)?;
        let owner = u64::from_le_bytes(random_bytes()?);
        Ok(Arc::new(Self {
            core: Mutex::new(Core {
                backend,
                state,
                ledger,
                limits,
                attached: false,
                attachment: None,
                verified: false,
                stopped: false,
                released: false,
                failed: false,
                session: None,
                proof: None,
                storage_claimed: false,
                issued: BTreeMap::new(),
                next_permit: 1,
            }),
            owner,
            prepare_lock: tokio::sync::Mutex::new(()),
            notification: Arc::new(tokio::sync::Notify::new()),
        }))
    }
    pub fn binding(&self) -> Result<AuthorizationBinding> {
        Ok(self
            .core
            .lock()
            .map_err(|_| Error::Storage)?
            .backend
            .binding()
            .clone())
    }
    pub(crate) fn verified_binding(&self) -> Result<AuthorizationBinding> {
        let core = self.core.lock().map_err(|_| Error::Storage)?;
        core.ready()?;
        if !core.verified {
            return Err(Error::Authorization);
        }
        Ok(core.backend.binding().clone())
    }
    pub async fn transport(
        self: &Arc<Self>,
        client: grammers_client::Client,
    ) -> Result<crate::transport::GrammersTransport> {
        self.verify_client(&client).await?;
        Ok(crate::transport::GrammersTransport::from_verified_ingress(
            client,
            Arc::clone(self),
        ))
    }
    /// `baseline` is an explicit previously verified MessageBoxes snapshot.
    /// A reconstructed journal refuses replacement baselines.
    pub async fn prepare(
        self: &Arc<Self>,
        pool: &SenderPoolFatHandle,
        baseline: Option<Vec<u8>>,
    ) -> Result<()> {
        let _serial = self.prepare_lock.lock().await;
        {
            let mut core = self.core.lock().map_err(|_| Error::Storage)?;
            if core.attached || core.stopped || core.released {
                return Err(Error::IngressUnavailable);
            }
            core.session = Some(Arc::clone(&pool.session));
            core.check_session()?;
        }
        let known = pool
            .session
            .peer(PeerId::self_user())
            .await
            .map_err(|_| Error::Authorization)?;
        let binding = self.binding()?;
        if let Some(peer) = known
            && !matches!(peer,PeerInfo::User{id,..} if id==binding.user_id())
        {
            return Err(Error::Authorization);
        }
        {
            let mut core = self.core.lock().map_err(|_| Error::Storage)?;
            let mut ledger = core.ledger.clone();
            match (ledger.checkpoint.is_some(), baseline) {
                (false, Some(baseline)) => {
                    ledger.bootstrap(baseline)?;
                }
                (true, None) => {}
                _ => return Err(Error::IngressUnavailable),
            }
            core.attachment = Some(ledger);
        }
        if pool
            .install_durable_ingress(Arc::clone(self) as Arc<dyn DurableIngressHook>)
            .is_err()
        {
            let mut core = self.core.lock().map_err(|_| Error::Storage)?;
            core.attached = false;
            core.attachment = None;
            return Err(Error::IngressUnavailable);
        }
        Ok(())
    }
    /// Prove the exact Client uses the prepared original pool. No new Client,
    /// connection, session import or authorization is constructed here.
    pub async fn verify_client(&self, client: &grammers_client::Client) -> Result<()> {
        let nonce = i64::from_le_bytes(random_bytes()?);
        {
            let mut core = self.core.lock().map_err(|_| Error::Storage)?;
            core.ready()?;
            if core.verified || core.proof.is_some() {
                return Err(Error::IngressUnavailable);
            }
            core.proof = Some((nonce, false));
        }
        struct ProofGuard<'a> {
            ingress: &'a DurableIngress,
            nonce: i64,
        }
        impl Drop for ProofGuard<'_> {
            fn drop(&mut self) {
                if let Ok(mut core) = self.ingress.core.lock()
                    && core.proof.is_some_and(|proof| proof.0 == self.nonce)
                {
                    core.proof = None;
                }
            }
        }
        let _proof = ProofGuard {
            ingress: self,
            nonce,
        };
        let result = client.invoke(&tl::functions::Ping { ping_id: nonce }).await;
        let observed = {
            let core = self.core.lock().map_err(|_| Error::Storage)?;
            core.proof == Some((nonce, true))
        };
        if !observed || !matches!(result,Ok(tl::enums::Pong::Pong(pong)) if pong.ping_id==nonce) {
            self.core.lock().map_err(|_| Error::Storage)?.proof = None;
            return Err(Error::Authorization);
        }
        let users = client
            .invoke(&tl::functions::users::GetUsers {
                id: vec![tl::enums::InputUser::UserSelf],
            })
            .await
            .map_err(|_| Error::Authorization)?;
        let mut core = self.core.lock().map_err(|_| Error::Storage)?;
        core.ready()?;
        if !matches!(users.as_slice(),[tl::enums::User::User(user)] if user.is_self&&!user.bot&&user.id==core.backend.binding().user_id())
        {
            core.proof = None;
            return Err(Error::Authorization);
        }
        core.verified = true;
        core.proof = None;
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<Vec<u8>> {
        Ok(self
            .core
            .lock()
            .map_err(|_| Error::Storage)?
            .ledger
            .checkpoint()?
            .to_vec())
    }
    pub fn pending_batches(&self) -> Result<Vec<IngressBatch>> {
        Ok(self
            .core
            .lock()
            .map_err(|_| Error::Storage)?
            .ledger
            .batches
            .clone())
    }
    pub fn limits(&self) -> Result<IngressLimits> {
        Ok(self.core.lock().map_err(|_| Error::Storage)?.limits.clone())
    }
    /// Non-secret updates have no secret-protocol mutation to couple. This does
    /// not settle any external cloud-send/callback obligation.
    pub fn acknowledge_nonsecret(&self, batch: u64, index: usize) -> Result<()> {
        self.acknowledge_without_protocol(batch, index, false, false)
    }
    /// The manager must supply its actual in-flight creation status. Known
    /// chats and quarantined early terminal events require protocol application.
    pub fn acknowledge_unknown_discarded(
        &self,
        batch: u64,
        index: usize,
        creation_in_flight: bool,
    ) -> Result<()> {
        self.acknowledge_without_protocol(batch, index, true, creation_in_flight)
    }
    fn acknowledge_without_protocol(
        &self,
        batch: u64,
        index: usize,
        terminal: bool,
        creating: bool,
    ) -> Result<()> {
        let mut core = self.core.lock().map_err(|_| Error::Storage)?;
        if core.stopped || core.released {
            return Err(Error::IngressUnavailable);
        }
        let row = core
            .ledger
            .batches
            .iter()
            .find(|row| row.id == batch)
            .ok_or(Error::InvalidState)?;
        let output = row.updates()?;
        let update = &output.0.get(index).ok_or(Error::InvalidState)?.0;
        if terminal {
            if creating {
                return Err(Error::IngressUnavailable);
            }
            match update {
                tl::enums::Update::Encryption(update) => match &update.chat {
                    tl::enums::EncryptedChat::Discarded(chat)
                        if chat.id > 0 && !core.state.chats.contains_key(&chat.id) => {}
                    _ => return Err(Error::IngressUnavailable),
                },
                _ => return Err(Error::IngressUnavailable),
            }
        } else if matches!(
            update,
            tl::enums::Update::Encryption(_) | tl::enums::Update::NewEncryptedMessage(_)
        ) {
            return Err(Error::IngressUnavailable);
        }
        let hash = sha2::Sha256::digest(update.to_bytes()).into();
        let mut ledger = core.ledger.clone();
        ledger.mark(batch, index, &hash)?;
        let state = core.state.clone();
        core.persist(state, ledger)
    }
    pub fn fence(&self) -> Result<()> {
        let mut core = self.core.lock().map_err(|_| Error::Storage)?;
        core.stopped = true;
        core.proof = None;
        core.issued.clear();
        self.notification.notify_one();
        Ok(())
    }
    pub fn protocol_storage(self: &Arc<Self>) -> Result<super::shared::IngressProtocolStorage> {
        let mut core = self.core.lock().map_err(|_| Error::Storage)?;
        core.ready()?;
        if core.storage_claimed {
            return Err(Error::IngressUnavailable);
        }
        core.storage_claimed = true;
        Ok(super::shared::IngressProtocolStorage {
            coordinator: Arc::clone(self),
        })
    }
    pub async fn apply<F: Future>(&self, batch: u64, index: usize, future: F) -> Result<F::Output> {
        let (hash, kind, chat_id) = {
            let core = self.core.lock().map_err(|_| Error::Storage)?;
            let row = core
                .ledger
                .batches
                .iter()
                .find(|row| row.id == batch)
                .ok_or(Error::InvalidState)?;
            let output = row.updates()?;
            let update = &output.0.get(index).ok_or(Error::InvalidState)?.0;
            use sha2::Sha256;
            let (kind, id) = match update {
                tl::enums::Update::NewEncryptedMessage(update) => (
                    application::Kind::Message,
                    match &update.message {
                        tl::enums::EncryptedMessage::Message(message) => message.chat_id,
                        tl::enums::EncryptedMessage::Service(message) => message.chat_id,
                    },
                ),
                tl::enums::Update::Encryption(update) => {
                    (application::Kind::Encryption, update.chat.id())
                }
                _ => (application::Kind::Other, 0),
            };
            (
                <[u8; 32]>::from(Sha256::digest(update.to_bytes())),
                kind,
                id,
            )
        };
        Ok(application::scope(
            application::Application::new(self.owner, batch, index, hash, kind, chat_id),
            future,
        )
        .await)
    }
    pub fn prune_completed(&self) -> Result<()> {
        let mut core = self.core.lock().map_err(|_| Error::Storage)?;
        let mut ledger = core.ledger.clone();
        ledger.prune_completed()?;
        let state = core.state.clone();
        core.persist(state, ledger)
    }
}
