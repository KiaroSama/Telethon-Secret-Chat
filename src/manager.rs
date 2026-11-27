use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tokio::sync::Mutex;

use crate::errors::SecretChatError;
use crate::protocol::{
    AcceptedMessage, ChatRole, ChatState, SecretChat, encrypt_frame, outgoing_layer, wrap_frame,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Caller-owned transport boundary: production callers supply their own authorized
/// `grammers` client adapter; offline parity tests supply a deterministic fake.
pub trait SecretTransport: Send + Sync {
    fn auth_identity(&self) -> &str;
    fn send_encrypted<'a>(
        &'a self,
        chat_id: i64,
        random_id: i64,
        frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>>;
    fn discard_encryption<'a>(
        &'a self,
        chat_id: i64,
        delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretEvent {
    ChatReady {
        chat_id: i64,
    },
    MessageReceived {
        chat_id: i64,
        message: AcceptedMessage,
    },
    ChatClosed {
        chat_id: i64,
        reason: String,
    },
}

/// Native async secret-chat manager owning per-chat state, locking, outbound framing,
/// inbound verification, and clean shutdown over a caller-owned `SecretTransport`.
pub struct SecretChatManager<T: SecretTransport> {
    transport: Arc<T>,
    chats: Mutex<BTreeMap<i64, SecretChat>>,
    events: Mutex<Vec<SecretEvent>>,
    stopping: AtomicBool,
}

impl<T: SecretTransport> SecretChatManager<T> {
    pub fn new(transport: Arc<T>) -> Self {
        Self {
            transport,
            chats: Mutex::new(BTreeMap::new()),
            events: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
        }
    }

    pub fn caller_auth_identity(&self) -> &str {
        self.transport.auth_identity()
    }

    pub async fn register_established_chat(
        &self,
        chat_id: i64,
        access_hash: i64,
        peer_user_id: i64,
        role: ChatRole,
        shared_key: &[u8],
    ) -> Result<i64, SecretChatError> {
        self.ensure_running()?;
        let mut chat = SecretChat::new(
            chat_id,
            access_hash,
            peer_user_id,
            self.transport.auth_identity(),
            role,
        );
        let fp = chat.adopt_key(shared_key)?;
        self.chats.lock().await.insert(chat_id, chat);
        self.events
            .lock()
            .await
            .push(SecretEvent::ChatReady { chat_id });
        Ok(fp)
    }

    pub async fn send_text(
        &self,
        chat_id: i64,
        random_id: i64,
        text: &str,
        ttl: i64,
    ) -> Result<Vec<u8>, SecretChatError> {
        self.ensure_running()?;
        let (frame, next_out_seq) = {
            let chats = self.chats.lock().await;
            let chat = chats
                .get(&chat_id)
                .ok_or(SecretChatError::UnknownChat { chat_id })?;
            if chat.state() == ChatState::Closed {
                return Err(SecretChatError::ChatClosed {
                    chat_id,
                    reason: "chat is closed".into(),
                });
            }
            if chat.state() != ChatState::Ready {
                return Err(SecretChatError::ChatNotReady {
                    chat_id,
                    state: format!("{:?}", chat.state()).to_lowercase(),
                });
            }
            let layer = outgoing_layer(chat.negotiated_layer())?;
            let in_wire = chat.in_seq_no() * 2 + i64::from(!chat.is_outbound());
            let out_wire = chat.out_seq_no() * 2 + i64::from(chat.is_outbound());
            let body = wrap_frame(
                &json!({
                    "random_id": random_id,
                    "ttl": ttl,
                    "message": text,
                }),
                layer,
                in_wire,
                out_wire,
            )?;
            let key = self.active_key_for_send(chat_id, chat)?;
            let frame = encrypt_frame(&key, &body, chat.out_x())?;
            (frame, chat.out_seq_no() + 1)
        };

        // Transport send runs outside the chat map lock; failures surface SendPending without advancing out_seq_no.
        self.transport
            .send_encrypted(chat_id, random_id, frame.clone())
            .await
            .map_err(|_| SecretChatError::SendPending {
                chat_id,
                random_id,
                cause: "TransportError".into(),
            })?;

        if let Some(chat) = self.chats.lock().await.get_mut(&chat_id) {
            chat.set_out_seq_no(next_out_seq);
        }
        Ok(frame)
    }

    pub async fn receive_frame(
        &self,
        chat_id: i64,
        frame: &[u8],
    ) -> Result<AcceptedMessage, SecretChatError> {
        self.ensure_running()?;
        let accepted = {
            let mut chats = self.chats.lock().await;
            let chat = chats
                .get_mut(&chat_id)
                .ok_or(SecretChatError::UnknownChat { chat_id })?;
            chat.receive_encrypted_frame(frame)?
        };
        self.events.lock().await.push(SecretEvent::MessageReceived {
            chat_id,
            message: accepted.clone(),
        });
        Ok(accepted)
    }

    pub async fn close_chat(
        &self,
        chat_id: i64,
        delete_history: bool,
        reason: &str,
    ) -> Result<(), SecretChatError> {
        let mut chats = self.chats.lock().await;
        let chat = chats
            .get_mut(&chat_id)
            .ok_or(SecretChatError::UnknownChat { chat_id })?;
        chat.close();
        drop(chats);

        let result = self
            .transport
            .discard_encryption(chat_id, delete_history)
            .await;
        self.events.lock().await.push(SecretEvent::ChatClosed {
            chat_id,
            reason: reason.to_owned(),
        });
        result
    }

    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let mut chats = self.chats.lock().await;
        for chat in chats.values_mut() {
            chat.close();
        }
    }

    pub async fn drain_events(&self) -> Vec<SecretEvent> {
        let mut events = self.events.lock().await;
        std::mem::take(&mut *events)
    }

    pub async fn chat_state(&self, chat_id: i64) -> Result<ChatState, SecretChatError> {
        self.chats
            .lock()
            .await
            .get(&chat_id)
            .map(SecretChat::state)
            .ok_or(SecretChatError::UnknownChat { chat_id })
    }

    pub async fn out_seq_no(&self, chat_id: i64) -> Result<i64, SecretChatError> {
        self.chats
            .lock()
            .await
            .get(&chat_id)
            .map(SecretChat::out_seq_no)
            .ok_or(SecretChatError::UnknownChat { chat_id })
    }

    fn ensure_running(&self) -> Result<(), SecretChatError> {
        if self.stopping.load(Ordering::SeqCst) {
            return Err(SecretChatError::ManagerStopping);
        }
        Ok(())
    }

    fn active_key_for_send(
        &self,
        chat_id: i64,
        chat: &SecretChat,
    ) -> Result<[u8; 256], SecretChatError> {
        chat.export_key_for_internal_frame()
            .ok_or(SecretChatError::ChatNotReady {
                chat_id,
                state: format!("{:?}", chat.state()).to_lowercase(),
            })
    }
}
