use super::{Failure, HUMAN, Pump, Refused, TestResult, require};
use crate::support_live::{Recorder, ready_chat};
use kiaro_secret_chat::{
    grammers_client::Client,
    grammers_mtsender::{SenderPool, SenderPoolFatHandle, UpdatesConfiguration, UpdatesReceiver},
    grammers_session::{
        SessionData,
        storages::MemorySession,
        types::{PeerId, PeerInfo, UpdateState, UpdatesState},
    },
    grammers_tl_types as tl,
    manager::{Options, SecretChatManager},
    storage::FileStorage,
    transport::GrammersTransport,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;
use zeroize::{Zeroize, Zeroizing};
pub type Manager = SecretChatManager<FileStorage, GrammersTransport>;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub api_id: i32,
    pub user_id: i64,
    pub dc_id: i32,
    auth_key: String,
    pub peer_user_id: i64,
    peer_access_hash: i64,
}
impl Drop for Config {
    fn drop(&mut self) {
        self.auth_key.zeroize();
    }
}
impl Config {
    pub fn load(name: &str) -> TestResult<Self> {
        require(
            std::env::var("TSC_RUN_LIVE").ok().as_deref() == Some("1"),
            "Live account tests require explicit TSC_RUN_LIVE=1 and a dedicated idle test authorization",
        )?;
        let path = PathBuf::from(std::env::var_os(name).ok_or(Refused(
            "An explicitly configured live session file is required",
        ))?);
        require(
            !path.ancestors().any(|path| {
                std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
            }),
            "Live authorization input must not use symbolic links",
        )?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let file = options.open(path).map_err(|_| {
            Refused("The explicitly configured live session file could not be opened")
        })?;
        let metadata = file
            .metadata()
            .map_err(|_| Refused("Live authorization metadata could not be inspected"))?;
        require(
            metadata.is_file() && metadata.len() <= 16 * 1024,
            "The live session document has an invalid size or type",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            require(
                metadata.permissions().mode() & 0o077 == 0,
                "The live session document must have owner-only permissions",
            )?;
        }
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(16 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Refused("The live session document could not be read"))?;
        require(
            bytes.len() <= 16 * 1024,
            "The live session document exceeded its read bound",
        )?;
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| {
            Refused("The live session document is invalid; its contents were withheld")
        })?;
        require(
            value.api_id > 0
                && value.user_id > 0
                && value.peer_user_id > 0
                && value.user_id != value.peer_user_id
                && (1..=5).contains(&value.dc_id)
                && value.auth_key.len() == 512,
            "The dedicated live session identity or key shape is invalid",
        )?;
        Ok(value)
    }
    pub fn peer(&self) -> tl::enums::InputUser {
        tl::types::InputUser {
            user_id: self.peer_user_id,
            access_hash: self.peer_access_hash,
        }
        .into()
    }
}
pub struct Account {
    pub manager: Manager,
    pub client: Client,
    pub recorder: Recorder,
    pub fingerprints: Arc<Mutex<BTreeMap<i32, i64>>>,
    pub scope: Arc<Mutex<BTreeSet<i32>>>,
    pub config: Config,
    pub(super) handle: SenderPoolFatHandle,
    pump: Option<Pump>,
    runner: Option<JoinHandle<()>>,
    directory: tempfile::TempDir,
    closed: bool,
}
struct RunnerGuard {
    handle: SenderPoolFatHandle,
    task: Option<JoinHandle<()>>,
}
impl Drop for RunnerGuard {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            self.handle.quit();
            task.abort();
        }
    }
}
impl Account {
    pub async fn open(config: Config, accept: bool) -> TestResult<Self> {
        tokio::time::timeout(Duration::from_secs(90), Self::open_inner(config, accept))
            .await
            .map_err(|_| {
                Refused("The explicitly requested live account setup exceeded its deadline")
            })?
    }
    async fn open_inner(config: Config, accept: bool) -> TestResult<Self> {
        let mut key = Zeroizing::new([0u8; 256]);
        hex::decode_to_slice(&config.auth_key, &mut key[..])
            .map_err(|_| Refused("The configured authorization key encoding is invalid"))?;
        let mut session = SessionData {
            home_dc: config.dc_id,
            ..Default::default()
        };
        session
            .dc_options
            .get_mut(&config.dc_id)
            .ok_or(Refused("The configured datacenter is unavailable"))?
            .auth_key = Some(*key);
        session.peer_infos.insert(
            PeerId::self_user(),
            PeerInfo::User {
                id: config.user_id,
                auth: None,
                bot: Some(false),
                is_self: Some(true),
            },
        );
        let pool = SenderPool::new(Arc::new(MemorySession::from(session)), config.api_id);
        let client = Client::new(pool.handle.clone());
        let mut runner = RunnerGuard {
            handle: pool.handle.clone(),
            task: Some(tokio::spawn(pool.runner.run())),
        };
        let users=client.invoke(&tl::functions::users::GetUsers{id:vec![tl::enums::InputUser::UserSelf]}).await.map_err(|_|Refused("The existing live authorization could not be verified; no sign-in will be attempted"))?;
        require(
            matches!(users.as_slice(),[tl::enums::User::User(user)] if user.is_self&&!user.bot&&user.id==config.user_id),
            "The live authorization does not belong to the configured non-bot test account",
        )?;
        let tl::enums::updates::State::State(state) = client
            .invoke(&tl::functions::updates::GetState {})
            .await
            .map_err(|_| Refused("The live account update baseline could not be read"))?;
        pool.handle
            .session
            .set_update_state(UpdateState::All(UpdatesState {
                pts: state.pts,
                qts: state.qts,
                date: state.date,
                seq: state.seq,
                channels: vec![],
            }))
            .await
            .map_err(|_| {
                Refused("The caller's in-memory update baseline could not be initialized")
            })?;
        let directory = tempfile::tempdir()
            .map_err(|_| Refused("The private live-test directory could not be created"))?;
        let manager = Manager::verified(
            GrammersTransport::new(client.clone()),
            FileStorage::open(directory.path().join("chat-state.json"))?,
            Options::default(),
        )
        .await?;
        manager.stop_auto_save_secret_chats().await?;
        let recorder = Recorder::default();
        record(&manager, &recorder)?;
        manager.start().await?;
        let receiver = UpdatesReceiver::create(
            pool.handle.thin.clone(),
            pool.handle.session.clone(),
            pool.updates,
            UpdatesConfiguration { catch_up: true },
        )
        .await
        .map_err(|_| Refused("The original caller update receiver could not be created"))?;
        let fingerprints = Arc::new(Mutex::new(BTreeMap::new()));
        let scope = Arc::new(Mutex::new(BTreeSet::new()));
        let pump = Pump::spawn(
            receiver,
            manager.clone(),
            recorder.clone(),
            Arc::clone(&fingerprints),
            Arc::clone(&scope),
            config.user_id,
            config.peer_user_id,
            accept,
        );
        let task = runner.task.take();
        // Move the original runner ownership out of the setup guard.
        let handle = pool.handle;
        drop(runner);
        Ok(Self {
            manager,
            client,
            recorder,
            fingerprints,
            scope,
            config,
            handle,
            pump: Some(pump),
            runner: task,
            directory,
            closed: false,
        })
    }
    pub async fn ready(&self, label: &str) -> TestResult<i32> {
        let (chat, _) = ready_chat(
            &self.manager,
            &self.recorder,
            self.config.peer(),
            |message| println!("{message}"),
            label,
            HUMAN,
        )
        .await?;
        self.scope
            .lock()
            .map_err(|_| Refused("Live ownership tracking failed"))?
            .insert(chat.id);
        Ok(chat.id)
    }
    pub async fn restart(&mut self) -> TestResult {
        let pump = self
            .pump
            .take()
            .ok_or(Refused("The caller update pump is unavailable"))?;
        let receiver = pump.pause().await?;
        self.manager.stop().await?;
        self.manager = Manager::verified(
            GrammersTransport::new(self.client.clone()),
            FileStorage::open(self.directory.path().join("chat-state.json"))?,
            Options::default(),
        )
        .await?;
        self.manager.stop_auto_save_secret_chats().await?;
        self.recorder = Recorder::default();
        record(&self.manager, &self.recorder)?;
        self.manager.start().await?;
        self.pump = Some(Pump::spawn(
            receiver,
            self.manager.clone(),
            self.recorder.clone(),
            Arc::clone(&self.fingerprints),
            Arc::clone(&self.scope),
            self.config.user_id,
            self.config.peer_user_id,
            true,
        ));
        Ok(())
    }
    pub async fn shutdown(&mut self) -> TestResult {
        if self.closed {
            return Ok(());
        }
        let mut failure: Option<Failure> = None;
        for chat in self.manager.list()? {
            if chat.state != kiaro_secret_chat::chat::ChatState::Closed {
                match tokio::time::timeout(Duration::from_secs(15),self.manager.close(chat.id)).await {
                    Ok(Ok(()))=>{},
                    Ok(Err(error))=>failure=Some(error.into()),
                    Err(_)=>failure=Some(Refused("Live chat cleanup exceeded its deadline; verify the throwaway chat manually").into()),
                }
            }
        }
        if let Some(pump) = self.pump.take() {
            match tokio::time::timeout(Duration::from_secs(15), pump.pause()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => failure = Some(error),
                Err(_) => {
                    failure = Some(
                        Refused("The live update pump did not settle before its deadline").into(),
                    )
                }
            }
        }
        match tokio::time::timeout(Duration::from_secs(15), self.manager.stop()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(error.into()),
            Err(_) => {
                failure = Some(
                    Refused("The live manager did not settle before its cleanup deadline").into(),
                )
            }
        }
        self.handle.quit();
        if let Some(mut task) = self.runner.take() {
            match tokio::time::timeout(Duration::from_secs(15), &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    failure = Some(Refused("The original live runner failed during cleanup").into())
                }
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    failure = Some(
                        Refused(
                            "The original live runner did not stop within its cleanup deadline",
                        )
                        .into(),
                    );
                }
            }
        }
        self.closed = true;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }
}
impl Drop for Account {
    fn drop(&mut self) {
        if !self.closed {
            if let Some(pump) = self.pump.take() {
                drop(pump);
            }
            self.handle.quit();
            if let Some(task) = &self.runner {
                task.abort();
            }
            eprintln!(
                "Live test interrupted; verify cleanup of its throwaway chats before another run."
            );
        }
    }
}
fn record(manager: &Manager, recorder: &Recorder) -> TestResult {
    for name in [
        "ChatRequested",
        "ChatReady",
        "ChatClosedEvent",
        "MessageReceived",
        "MessageAcknowledged",
        "ServiceActionReceived",
        "DecryptFailed",
        "SendFailed",
    ] {
        let recorder = recorder.clone();
        manager.on(name, "live-observation", move |event| {
            recorder.record(event);
            Ok(())
        })?;
    }
    Ok(())
}
