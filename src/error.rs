//! Errors use fixed messages; network responses, paths and key material are excluded.
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Operating-system randomness is unavailable")]
    Randomness,
    #[error("A shared key must contain exactly 256 bytes")]
    KeyLength,
    #[error("The ciphertext frame was rejected")]
    FrameRejected,
    #[error("The secret-chat wrapper was rejected")]
    WrapperRejected,
    #[error("Diffie-Hellman parameters were rejected")]
    DhParameters,
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

pub type Result<T> = std::result::Result<T, Error>;
