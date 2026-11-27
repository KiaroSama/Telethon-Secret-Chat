use thiserror::Error;

/// Bounded, secret-free protocol and custody errors (Constitution Principle IV).
///
/// Every variant carries only structural context (chat id, state name, numeric
/// bounds, or a fixed safe reason) and never interpolates keys, plaintext,
/// ciphertext, peer public values, or raw protocol object reprs.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretChatError {
    #[error("{}", format_chat_reason("parameter rejected", *.chat_id, .reason))]
    ParameterRejected {
        chat_id: Option<i64>,
        reason: String,
    },

    #[error("{}", format_chat_reason("rejected a received message", *.chat_id, .reason))]
    MessageRejected {
        chat_id: Option<i64>,
        reason: String,
    },

    #[error("[chat {chat_id}] chat is not ready (state: {state})")]
    ChatNotReady { chat_id: i64, state: String },

    #[error("[chat {chat_id}] chat is closed: {reason}")]
    ChatClosed { chat_id: i64, reason: String },

    #[error("explicit durable storage is required for this custody mode")]
    StorageRequired,

    #[error("[chat {chat_id}] peer announced unsupported secret-chat layer {peer_layer}")]
    LayerUnsupported { chat_id: i64, peer_layer: i64 },

    #[error(
        "[chat {chat_id}] resend range ({}, {}) cannot be satisfied from retained sequence {retained_from}",
        .requested.0,
        .requested.1
    )]
    ResendUnsatisfiable {
        chat_id: i64,
        requested: (i64, i64),
        retained_from: i64,
    },

    #[error("{}", format_chat_reason("stored secret-chat state is corrupt", *.chat_id, .reason))]
    StoreCorrupt {
        chat_id: Option<i64>,
        reason: String,
    },

    #[error("[chat {chat_id}] unknown secret chat")]
    UnknownChat { chat_id: i64 },

    #[error("secret-chat manager is stopping")]
    ManagerStopping,

    #[error("[chat {chat_id}] outbound message {random_id} delivery pending ({cause})")]
    SendPending {
        chat_id: i64,
        random_id: i64,
        cause: String,
    },

    #[error("durable ingress refused: {reason}")]
    IngressRefused { reason: String },

    #[error("operating system entropy source unavailable: {reason}")]
    EntropyUnavailable { reason: String },

    #[error("secret-chat protocol error: {0}")]
    Protocol(&'static str),
}

fn format_chat_reason(label: &str, chat_id: Option<i64>, reason: &str) -> String {
    match chat_id {
        Some(id) => format!("[chat {id}] {label}: {reason}"),
        None => format!("{label}: {reason}"),
    }
}
