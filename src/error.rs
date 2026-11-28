//! Protocol and storage refusals use fixed messages. Caller-facing media input
//! validation can name supplied metadata; runtime logs must not format errors.
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Context(ChatContext),
    #[error("The media field {field} was rejected")]
    MediaField { field: &'static str },
    #[error("Unknown media kind {kind}")]
    MediaKind { kind: String },
    #[error("The file {file_name} cannot be sent as {kind}")]
    MediaKindMismatch {
        file_name: String,
        kind: &'static str,
    },
    #[error("The media kind {kind} does not support captions")]
    MediaCaption { kind: &'static str },
    #[error("A committed send is pending; retry its original identity")]
    SendPending {
        chat_id: i32,
        random_id: i64,
        cause: PendingCause,
    },
    #[error("The manager is stopping")]
    ManagerStopping,
    #[error("No secret chat with this identity is held")]
    UnknownChat { chat_id: i32 },
    #[error("The secret chat has ended")]
    ChatClosed { chat_id: i32 },
    #[error("The secret chat is not ready")]
    ChatNotReady { chat_id: i32 },
    #[error("Operating-system randomness is unavailable")]
    Randomness,
    #[error("A shared key must contain exactly 256 bytes")]
    KeyLength,
    #[error("The ciphertext frame was rejected")]
    FrameRejected,
    #[error("The secret-chat wrapper was rejected")]
    WrapperRejected,
    #[error("The authenticated inner secret-chat message was rejected")]
    InnerMessageRejected,
    #[error("Diffie-Hellman parameters were rejected")]
    DhParameters,
    #[error(transparent)]
    DhFailure(DhFailure),
    #[error("Secret chat {chat_id} is not ready ({state})")]
    ChatNotSendable {
        chat_id: i32,
        state: crate::chat::ChatState,
    },
    #[error("Secret chat {chat_id}: {reason}")]
    ChatEnded { chat_id: i32, reason: CloseReason },
    #[error("The peer public value was rejected")]
    PeerValue,
    #[error("The key fingerprint does not match")]
    Fingerprint,
    #[error("Secret-chat sequence invariants were violated")]
    Sequence,
    #[error("The requested resend span is unavailable")]
    ResendUnavailable,
    #[error("The chat is not in the required state")]
    ChatState,
    #[error("The manager is not running")]
    NotRunning,
    #[error("The conversation is paused")]
    Paused,
    #[error("The custody budget is exhausted")]
    Capacity,
    #[error("The protected state could not be read or committed")]
    Storage,
    #[error("The stored state is incompatible or invalid")]
    InvalidState,
    #[error("The media metadata was rejected")]
    Media,
    #[error("The Telegram request did not complete; a retained send may be retried")]
    Transport,
    #[error("The authorization does not match the protected state")]
    Authorization,
    #[error("This grammers transport has no verified pre-ack ingress hook")]
    IngressUnavailable,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DhFailure {
    #[error("The prime must have exactly 2048 bits")]
    Length,
    #[error("The generator must satisfy 1 < g < p - 1 and belong to 2 through 7")]
    GeneratorRange,
    #[error("The generator subgroup residue condition was rejected")]
    Residue,
    #[error("The modulus is not a safe prime")]
    UnsafePrime,
}
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    #[error("the peer discarded the chat")]
    PeerDiscarded,
    #[error("closed by this application")]
    Application,
    #[error("the protocol rejected further participation")]
    Protocol,
    #[error("rekey commitment failed its fingerprint check")]
    RekeyFingerprint,
    #[error("stored chat lacks its DH parameters and cannot rekey")]
    MissingDh,
    #[error("already closed")]
    AlreadyClosed,
}
/// Fixed classifications preserve retry diagnostics without retaining a foreign
/// exception that could contain plaintext, paths, or request metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingCause {
    Transport,
    Cancelled,
    Deadline,
    Storage,
}
impl PendingCause {
    pub fn name(self) -> &'static str {
        match self {
            Self::Transport => "TransportError",
            Self::Cancelled | Self::Deadline => "CancelledError",
            Self::Storage => "StorageError",
        }
    }
}

/// Optional conversation context for low-level, account-independent operations.
/// Construction is sealed so metadata validation cannot become an error source
/// that accidentally reveals a supplied filename or media value.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("Secret chat {chat_id}: {source}")]
pub struct ChatContext {
    chat_id: i32,
    #[source]
    source: Box<Error>,
}
impl ChatContext {
    pub fn chat_id(&self) -> i32 {
        self.chat_id
    }
    pub fn refusal(&self) -> &Error {
        &self.source
    }
}
impl Error {
    pub fn with_chat(self, chat_id: i32) -> Self {
        let error = match self {
            Self::Context(context) => *context.source,
            error => error,
        };
        let error = match error {
            Self::MediaField { .. }
            | Self::MediaKind { .. }
            | Self::MediaKindMismatch { .. }
            | Self::MediaCaption { .. } => Self::Media,
            error => error,
        };
        Self::Context(ChatContext {
            chat_id,
            source: Box::new(error),
        })
    }
}

pub type Result<T> = std::result::Result<T, Error>;
