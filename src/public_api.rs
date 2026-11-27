/// Canonical mapping of all 48 frozen baseline `telethon_secret_chat.__all__` exports
/// to their native Rust `kiaro_secret_chat` equivalents (FR-001, SC-001, SC-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicExportEntry {
    pub baseline_name: &'static str,
    pub native_path: &'static str,
}

pub const PUBLIC_EXPORTS: &[PublicExportEntry] = &[
    PublicExportEntry {
        baseline_name: "SecretChatManager",
        native_path: "kiaro_secret_chat::manager::SecretChatManager",
    },
    PublicExportEntry {
        baseline_name: "BoundedSecretChatManager",
        native_path: "kiaro_secret_chat::custody::CustodyRecord",
    },
    PublicExportEntry {
        baseline_name: "EncryptedIngressStorage",
        native_path: "kiaro_secret_chat::ingress::EncryptedIngressStore",
    },
    PublicExportEntry {
        baseline_name: "IngressRefused",
        native_path: "kiaro_secret_chat::errors::SecretChatError::IngressRefused",
    },
    PublicExportEntry {
        baseline_name: "DurableIngress",
        native_path: "kiaro_secret_chat::ingress::DurableIngressController",
    },
    PublicExportEntry {
        baseline_name: "StrongSecretChatManager",
        native_path: "kiaro_secret_chat::ingress::DurableIngressController",
    },
    PublicExportEntry {
        baseline_name: "IngressLimits",
        native_path: "kiaro_secret_chat::ingress::IngressLimits",
    },
    PublicExportEntry {
        baseline_name: "IngressBatch",
        native_path: "kiaro_secret_chat::ingress::DurableIngressRecord",
    },
    PublicExportEntry {
        baseline_name: "CustodyLimits",
        native_path: "kiaro_secret_chat::custody::CustodyLimits",
    },
    PublicExportEntry {
        baseline_name: "ChatPaused",
        native_path: "kiaro_secret_chat::custody::CustodyRecord::is_paused",
    },
    PublicExportEntry {
        baseline_name: "PauseStatus",
        native_path: "kiaro_secret_chat::custody::CustodyRecord",
    },
    PublicExportEntry {
        baseline_name: "RelayDelivery",
        native_path: "kiaro_secret_chat::ingress::DurableIngressRecord",
    },
    PublicExportEntry {
        baseline_name: "TransientSecretChatManager",
        native_path: "kiaro_secret_chat::custody::CustodyMode::Transient",
    },
    PublicExportEntry {
        baseline_name: "TransientLimits",
        native_path: "kiaro_secret_chat::custody::CustodyLimits",
    },
    PublicExportEntry {
        baseline_name: "TransientRefused",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ParameterRejected",
    },
    PublicExportEntry {
        baseline_name: "TransientCleanupIncomplete",
        native_path: "kiaro_secret_chat::errors::SecretChatError::StoreCorrupt",
    },
    PublicExportEntry {
        baseline_name: "ProtectedFileStorage",
        native_path: "kiaro_secret_chat::custody::CustodyStore",
    },
    PublicExportEntry {
        baseline_name: "SecretChat",
        native_path: "kiaro_secret_chat::protocol::SecretChat",
    },
    PublicExportEntry {
        baseline_name: "ChatSnapshot",
        native_path: "kiaro_secret_chat::protocol::SecretChat",
    },
    PublicExportEntry {
        baseline_name: "ChatState",
        native_path: "kiaro_secret_chat::protocol::ChatState",
    },
    PublicExportEntry {
        baseline_name: "StorageBackend",
        native_path: "kiaro_secret_chat::custody::CustodyBackend",
    },
    PublicExportEntry {
        baseline_name: "MemoryStorage",
        native_path: "kiaro_secret_chat::custody::CustodyBackend::MemoryOnly",
    },
    PublicExportEntry {
        baseline_name: "FileStorage",
        native_path: "kiaro_secret_chat::custody::CustodyStore",
    },
    PublicExportEntry {
        baseline_name: "MEDIA_KINDS",
        native_path: "kiaro_secret_chat::public_api::MEDIA_KINDS",
    },
    PublicExportEntry {
        baseline_name: "CAPTIONLESS_KINDS",
        native_path: "kiaro_secret_chat::public_api::CAPTIONLESS_KINDS",
    },
    PublicExportEntry {
        baseline_name: "SecretChatError",
        native_path: "kiaro_secret_chat::errors::SecretChatError",
    },
    PublicExportEntry {
        baseline_name: "ParameterRejected",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ParameterRejected",
    },
    PublicExportEntry {
        baseline_name: "ChatNotReady",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ChatNotReady",
    },
    PublicExportEntry {
        baseline_name: "ChatClosed",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ChatClosed",
    },
    PublicExportEntry {
        baseline_name: "StorageRequired",
        native_path: "kiaro_secret_chat::errors::SecretChatError::StorageRequired",
    },
    PublicExportEntry {
        baseline_name: "LayerUnsupported",
        native_path: "kiaro_secret_chat::errors::SecretChatError::LayerUnsupported",
    },
    PublicExportEntry {
        baseline_name: "ResendUnsatisfiable",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ResendUnsatisfiable",
    },
    PublicExportEntry {
        baseline_name: "StoreCorrupt",
        native_path: "kiaro_secret_chat::errors::SecretChatError::StoreCorrupt",
    },
    PublicExportEntry {
        baseline_name: "UnknownChat",
        native_path: "kiaro_secret_chat::errors::SecretChatError::UnknownChat",
    },
    PublicExportEntry {
        baseline_name: "ManagerStopping",
        native_path: "kiaro_secret_chat::errors::SecretChatError::ManagerStopping",
    },
    PublicExportEntry {
        baseline_name: "SendPending",
        native_path: "kiaro_secret_chat::errors::SecretChatError::SendPending",
    },
    PublicExportEntry {
        baseline_name: "ChatRequested",
        native_path: "kiaro_secret_chat::protocol::ChatState::Requested",
    },
    PublicExportEntry {
        baseline_name: "ChatReady",
        native_path: "kiaro_secret_chat::protocol::ChatState::Ready",
    },
    PublicExportEntry {
        baseline_name: "ChatClosedEvent",
        native_path: "kiaro_secret_chat::protocol::ChatState::Closed",
    },
    PublicExportEntry {
        baseline_name: "MessageReceived",
        native_path: "kiaro_secret_chat::protocol::AcceptedMessage",
    },
    PublicExportEntry {
        baseline_name: "MessageAcknowledged",
        native_path: "kiaro_secret_chat::protocol::SecretChat::peer_in_seq_no",
    },
    PublicExportEntry {
        baseline_name: "ServiceActionReceived",
        native_path: "kiaro_secret_chat::protocol::AcceptedMessage",
    },
    PublicExportEntry {
        baseline_name: "DecryptFailed",
        native_path: "kiaro_secret_chat::errors::SecretChatError::MessageRejected",
    },
    PublicExportEntry {
        baseline_name: "SendFailed",
        native_path: "kiaro_secret_chat::errors::SecretChatError::SendPending",
    },
    PublicExportEntry {
        baseline_name: "MediaReference",
        native_path: "kiaro_secret_chat::protocol::AcceptedMessage",
    },
    PublicExportEntry {
        baseline_name: "KeyVisualization",
        native_path: "kiaro_secret_chat::protocol::key_visualization",
    },
    PublicExportEntry {
        baseline_name: "key_visualization",
        native_path: "kiaro_secret_chat::protocol::key_visualization",
    },
    PublicExportEntry {
        baseline_name: "PALETTE",
        native_path: "kiaro_secret_chat::public_api::PALETTE",
    },
];

pub const MEDIA_KINDS: [&str; 7] = [
    "photo",
    "document",
    "video",
    "audio",
    "voice",
    "sticker",
    "animation",
];

pub const CAPTIONLESS_KINDS: [&str; 1] = ["sticker"];

pub const PALETTE: [(u8, u8, u8); 4] = [
    (255, 255, 255),
    (213, 230, 243),
    (45, 87, 117),
    (47, 40, 41),
];
