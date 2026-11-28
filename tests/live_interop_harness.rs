#![forbid(unsafe_code)]

use std::{env, sync::Arc, time::Duration};

use grammers_client::{Client, client::UpdateStream};
use grammers_mtsender::{SenderPool, SenderPoolHandle, UpdatesConfiguration};
use grammers_session::{
    Session, SessionData,
    storages::MemorySession,
    types::{PeerId, PeerInfo, UpdateState, UpdatesState},
};
use grammers_tl_types as tl;
use kiaro_secret_chat::{
    establishment::{EstablishedChat, SecretChatEstablisher},
    manager::SecretTransport,
    transport::{EncryptedChatWire, GrammersTransport, IncomingWireUpdate},
};
use zeroize::Zeroizing;

const API_ID: &str = "TELEGRAM_INTEROP_API_ID";
const API_HASH: &str = "TELEGRAM_INTEROP_API_HASH";
const SESSION_A: &str = "TELEGRAM_INTEROP_SESSION_A";
const SESSION_B: &str = "TELEGRAM_INTEROP_SESSION_B";
const PEER_ID: &str = "TELEGRAM_INTEROP_PEER_USER_ID";
const GENERATION: &str = "live-interop";
const RPC_LIMIT: Duration = Duration::from_secs(30);

type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy)]
enum Mode {
    Pair,
    Official,
}

struct Credentials {
    api_id: i32,
    session_a: SessionData,
    session_b: Option<SessionData>,
    peer_id: Option<i64>,
}

fn session(raw: &str) -> Option<SessionData> {
    let (dc, encoded) = raw.split_once(':')?;
    let dc = dc.parse().ok()?;
    let mut data = SessionData {
        home_dc: dc,
        ..Default::default()
    };
    let mut key = Zeroizing::new([0; 256]);
    hex::decode_to_slice(encoded, &mut *key).ok()?;
    if key.iter().all(|byte| *byte == 0) {
        return None;
    }
    data.dc_options.get_mut(&dc)?.auth_key = Some(*key);
    Some(data)
}

impl Credentials {
    fn parse(mode: Mode, mut get: impl FnMut(&str) -> Result<Option<String>>) -> Result<Self> {
        let api_id = get(API_ID)?
            .and_then(|value| value.parse().ok())
            .filter(|id| *id > 0)
            .ok_or("TELEGRAM_INTEROP_API_ID must be a positive integer")?;
        let hash = get(API_HASH)?.map(Zeroizing::new);
        if !hash.as_ref().is_some_and(|value| {
            value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err("TELEGRAM_INTEROP_API_HASH must contain 32 hex digits");
        }
        // Pre-authorized sessions use the API ID at connection time. No login is attempted.
        let session_a = get(SESSION_A)?
            .map(Zeroizing::new)
            .and_then(|value| session(&value))
            .ok_or("TELEGRAM_INTEROP_SESSION_A must contain a known DC and a 256-byte hex key")?;
        let session_b = match mode {
            Mode::Pair => Some(
                get(SESSION_B)?
                    .map(Zeroizing::new)
                    .and_then(|value| session(&value))
                    .ok_or(
                        "TELEGRAM_INTEROP_SESSION_B must contain a known DC and a 256-byte hex key",
                    )?,
            ),
            Mode::Official => None,
        };
        if session_b.as_ref().is_some_and(|other| {
            other.dc_options[&other.home_dc].auth_key
                == session_a.dc_options[&session_a.home_dc].auth_key
        }) {
            return Err("Use separate Telegram authorizations for sessions A and B");
        }
        let peer_id = get(PEER_ID)?
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .parse()
                    .ok()
                    .filter(|id| PeerId::user(*id).is_some())
                    .ok_or("TELEGRAM_INTEROP_PEER_USER_ID must be a positive Telegram user ID")
            })
            .transpose()?;
        if matches!(mode, Mode::Official) && peer_id.is_none() {
            return Err("Official-client mode requires TELEGRAM_INTEROP_PEER_USER_ID");
        }
        Ok(Self {
            api_id,
            session_a,
            session_b,
            peer_id,
        })
    }

    fn from_env(mode: Mode) -> Result<Self> {
        Self::parse(mode, |name| match env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(env::VarError::NotUnicode(_)) => {
                Err("A Telegram interop variable is not valid UTF-8")
            }
        })
    }
}

#[test]
fn missing_credentials_refuse_before_any_network_operation() {
    fn credential(name: &str) -> Option<String> {
        match name {
            API_ID => Some("123".into()),
            API_HASH => Some("ab".repeat(16)),
            SESSION_A => Some(format!("2:{}", "05".repeat(256))),
            SESSION_B => Some(format!("2:{}", "06".repeat(256))),
            PEER_ID => Some("1234".into()),
            _ => None,
        }
    }
    for (missing, mode) in [
        (API_ID, Mode::Pair),
        (API_HASH, Mode::Pair),
        (SESSION_A, Mode::Pair),
        (SESSION_B, Mode::Pair),
        (PEER_ID, Mode::Official),
    ] {
        let error = Credentials::parse(mode, |name| {
            Ok((name != missing).then(|| credential(name)).flatten())
        })
        .err()
        .expect("missing credentials must be rejected without creating a client");
        assert!(error.contains(missing));
    }
}

#[test]
fn invalid_credentials_produce_fixed_errors_without_values() {
    let markers = ["secret-canary", "   ", "", "6:secret-canary"];
    for marker in markers {
        let result = Credentials::parse(Mode::Pair, |name| {
            Ok(Some(match name {
                API_ID => "123".into(),
                API_HASH => "ab".repeat(16),
                _ => marker.to_owned(),
            }))
        });
        assert_eq!(
            result.err(),
            Some("TELEGRAM_INTEROP_SESSION_A must contain a known DC and a 256-byte hex key")
        );
    }
    assert!(session(&format!("6:{}", "01".repeat(256))).is_none());
    assert!(session(&format!("2:{}", "00".repeat(256))).is_none());
}

struct Runner {
    handle: SenderPoolHandle,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.handle.quit();
        self.task.abort();
    }
}

struct Endpoint {
    client: Client,
    transport: GrammersTransport,
    updates: UpdateStream,
    user_id: i64,
    _runner: Runner,
}

impl Endpoint {
    async fn connect(session: SessionData, api_id: i32) -> Result<Self> {
        let session = Arc::new(MemorySession::from(session));
        let SenderPool {
            runner,
            handle,
            updates,
        } = SenderPool::new(Arc::clone(&session), api_id);
        let client = Client::new(handle.clone());
        let runner = Runner {
            handle: handle.thin,
            task: tokio::spawn(runner.run()),
        };
        let user = client
            .get_me()
            .await
            .map_err(|_| "Could not verify the controlled Telegram account")?;
        let user_id = user
            .id()
            .bare_id()
            .ok_or("Telegram did not return a user ID")?;
        if !user.is_self() || user.is_bot() {
            return Err("A controlled Telegram session must belong to a signed-in user");
        }
        let tl::enums::updates::State::State(state) = client
            .invoke(&tl::functions::updates::GetState {})
            .await
            .map_err(|_| "Could not read Telegram's starting update state")?;
        session
            .cache_peer(PeerInfo::User {
                id: user_id,
                auth: None,
                bot: Some(false),
                is_self: Some(true),
            })
            .await
            .map_err(|_| "Could not cache the controlled Telegram account")?;
        session
            .set_update_state(UpdateState::All(UpdatesState {
                pts: state.pts,
                qts: state.qts,
                date: state.date,
                seq: state.seq,
                ..Default::default()
            }))
            .await
            .map_err(|_| "Could not initialize Telegram update tracking")?;
        let transport = GrammersTransport::connect(client.clone(), GENERATION)
            .await
            .map_err(|_| "Could not bind the secret-chat transport")?;
        let updates = client
            .stream_updates(updates, UpdatesConfiguration { catch_up: true })
            .await
            .map_err(|_| "Could not start the Telegram update stream")?;
        Ok(Self {
            client,
            transport,
            updates,
            user_id,
            _runner: runner,
        })
    }

    async fn access_hash(&self, user_id: i64) -> Result<i64> {
        let target = PeerId::user(user_id).ok_or("Invalid peer user ID")?;
        let mut dialogs = self.client.iter_dialogs().limit(100);
        while let Some(dialog) = dialogs
            .next()
            .await
            .map_err(|_| "Could not read the controlled dialog list")?
        {
            if dialog.peer_id() == target {
                return dialog
                    .peer_ref()
                    .map(|peer| peer.auth.hash())
                    .ok_or("Controlled dialog has no usable peer reference");
            }
        }
        Err("Open a regular Telegram dialog with the controlled peer before running this test")
    }

    async fn update(&mut self) -> Result<IncomingWireUpdate> {
        loop {
            let (raw, _, _) = self
                .updates
                .next_raw()
                .await
                .map_err(|_| "Telegram update stream failed")?;
            if let Some(update) = GrammersTransport::adapt_update(&raw) {
                return Ok(update);
            }
        }
    }

    async fn chat_update(&mut self, id: i32, requested: bool) -> Result<EncryptedChatWire> {
        loop {
            match self.update().await? {
                IncomingWireUpdate::ChatUpdate(
                    wire @ EncryptedChatWire::Requested { chat_id, .. },
                ) if chat_id == id && requested => return Ok(wire),
                IncomingWireUpdate::ChatUpdate(
                    wire @ EncryptedChatWire::Active { chat_id, .. },
                ) if chat_id == id && !requested => return Ok(wire),
                IncomingWireUpdate::ChatUpdate(EncryptedChatWire::Discarded {
                    chat_id, ..
                }) if chat_id == id => return Err("Peer discarded the test chat"),
                _ => {}
            }
        }
    }

    async fn announce_layer(&self, chat: &mut EstablishedChat) -> Result<()> {
        let layer = serde_json::json!({"id": 0xf3048883_u32, "fields": {"layer": 144}});
        let service = chat
            .chat
            .prepare_service_action(&layer)
            .map_err(|_| "Could not prepare the layer announcement")?;
        SecretTransport::send_encrypted_service(
            &self.transport,
            chat.chat.chat_id(),
            service.random_id,
            service.frame,
        )
        .await
        .map_err(|_| "Telegram did not accept the layer announcement")
    }

    async fn send(&self, chat: &mut EstablishedChat, text: &str) -> Result<()> {
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(|_| "Could not generate a message ID")?;
        let id = i64::from_le_bytes(random);
        let id = if id == 0 { 1 } else { id };
        let frame = chat
            .encrypt_text(id, text, 0)
            .map_err(|_| "Could not encrypt the test message")?;
        SecretTransport::send_encrypted(&self.transport, chat.chat.chat_id(), id, frame)
            .await
            .map_err(|_| "Telegram did not accept the encrypted test message")
    }

    async fn receive(&mut self, chat: &mut EstablishedChat, expected: &str) -> Result<()> {
        loop {
            match self.update().await? {
                IncomingWireUpdate::NewEncryptedMessage { chat_id, bytes, .. }
                    if i64::from(chat_id) == chat.chat.chat_id() =>
                {
                    let messages = chat
                        .chat
                        .receive_encrypted_frames(&bytes)
                        .map_err(|_| "Could not decrypt an update for the test chat")?;
                    if messages
                        .iter()
                        .any(|message| message.text.as_deref() == Some(expected))
                    {
                        return Ok(());
                    }
                }
                IncomingWireUpdate::ChatUpdate(EncryptedChatWire::Discarded {
                    chat_id, ..
                }) if i64::from(chat_id) == chat.chat.chat_id() => {
                    return Err("Peer discarded the test chat");
                }
                _ => {}
            }
        }
    }
}

async fn deadline<T>(limit: Duration, work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| "Timed out waiting for the controlled Telegram peer or server")?
}

async fn run(mode: Mode) -> Result<()> {
    let Credentials {
        api_id,
        session_a,
        session_b,
        peer_id,
    } = Credentials::from_env(mode)?;
    let mut alice = deadline(RPC_LIMIT, Endpoint::connect(session_a, api_id)).await?;
    let mut bob = match session_b {
        Some(session) => Some(deadline(RPC_LIMIT, Endpoint::connect(session, api_id)).await?),
        None => None,
    };
    let peer = if let Some(bob) = &bob {
        if peer_id.is_some_and(|id| id != bob.user_id) {
            return Err("Configured peer does not match account B");
        }
        bob.user_id
    } else {
        peer_id.ok_or("Official-client mode requires a controlled peer ID")?
    };
    if peer == alice.user_id {
        return Err("Interop requires two distinct Telegram users");
    }
    let limit = match mode {
        Mode::Pair => Duration::from_secs(60),
        Mode::Official => Duration::from_secs(300),
    };
    let access_hash = deadline(RPC_LIMIT, alice.access_hash(peer)).await?;
    let initiator = SecretChatEstablisher::new(alice.transport.clone());
    let mut marker = [0; 16];
    getrandom::fill(&mut marker).map_err(|_| "Could not generate a test challenge")?;
    let prompt = format!(
        "Interop test. Reply with this code: {}",
        hex::encode(marker)
    );
    let answer = hex::encode(marker);
    let pending = deadline(RPC_LIMIT, async {
        initiator.request_chat(peer, access_hash).await.map_err(|_| "Could not request a controlled secret chat")
    }).await.inspect_err(|_| {
        eprintln!("If Telegram created a chat before confirming its request, close it from a controlled account.");
    })?;
    let id = pending.chat_id();
    let exchange = deadline(limit, async {
        let mut receiver = if let Some(bob) = &mut bob {
            let request = bob.chat_update(id, true).await?;
            let established = SecretChatEstablisher::new(bob.transport.clone())
                .accept_chat(&request).await.map_err(|_| "Account B could not accept the test chat")?;
            Some(established)
        } else {
            eprintln!("Accept the new secret chat in the official client, then reply with its challenge code.");
            None
        };
        let active = alice.chat_update(id, false).await?;
        let mut sender = initiator.complete_outgoing_handshake(pending, &active)
            .map_err(|_| "The controlled secret-chat handshake did not complete")?;
        alice.announce_layer(&mut sender).await?;
        if let (Some(bob), Some(receiver)) = (&mut bob, &mut receiver) {
            bob.announce_layer(receiver).await?;
        }
        alice.send(&mut sender, &prompt).await?;
        if let (Some(bob), Some(receiver)) = (&mut bob, &mut receiver) {
            if sender.key_picture.hex != receiver.key_picture.hex {
                return Err("The test parties derived different secret-chat keys");
            }
            bob.receive(receiver, &prompt).await?;
            bob.send(receiver, &answer).await?;
        }
        alice.receive(&mut sender, &answer).await
    }).await;
    let cleanup = deadline(RPC_LIMIT, async {
        SecretTransport::discard_encryption(&alice.transport, i64::from(id), false)
            .await
            .map_err(|_| "Could not close the controlled test chat")
    })
    .await;
    if cleanup.is_err() {
        eprintln!(
            "The temporary secret chat may still be open. Close it from a controlled account."
        );
    }
    exchange.and(cleanup)
}

#[tokio::test]
#[ignore = "opt-in live Telegram interoperability harness; requires controlled TELEGRAM_INTEROP_* credentials"]
async fn same_library_pair() -> Result<()> {
    run(Mode::Pair).await
}

#[tokio::test]
#[ignore = "opt-in live Telegram interoperability harness; requires controlled TELEGRAM_INTEROP_* credentials"]
async fn official_client_interop() -> Result<()> {
    run(Mode::Official).await
}
