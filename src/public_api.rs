//! Inventory of names from the retired Python public surface.
//!
//! This is migration metadata, not Rust re-exports or a compatibility layer.
//! A string naming an item does not establish signature or behavioral parity.
//! Import real APIs from their modules and consult the current architecture guide.

/// Whether an actual Rust item covers the named baseline capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustSurfaceStatus {
    /// A concrete item exists; its Rust signature and documented scope apply.
    NativeItem,
    /// A related item exists, but omits part of the baseline behavior.
    PartialCapability,
    /// The named type, event, or subsystem is not implemented in this crate.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicExportEntry {
    pub baseline_name: &'static str,
    /// A related compiled item, or an empty string when the capability is unavailable.
    /// This field does not create a type alias or importable baseline name.
    pub native_path: &'static str,
    pub status: RustSurfaceStatus,
    pub notes: &'static str,
}

const fn entry(
    baseline_name: &'static str,
    native_path: &'static str,
    status: RustSurfaceStatus,
    notes: &'static str,
) -> PublicExportEntry {
    PublicExportEntry {
        baseline_name,
        native_path,
        status,
        notes,
    }
}

use RustSurfaceStatus::{NativeItem, PartialCapability, Unavailable};

/// Historical baseline-name inventory with explicit current implementation status.
/// Its length is not a count of implemented Rust exports or proof of feature parity.
pub const PUBLIC_EXPORTS: &[PublicExportEntry] = &[
    entry(
        "SecretChatManager",
        "kiaro_secret_chat::runtime_modes::PersistentSecretChatManager",
        NativeItem,
        "Restartable secret-chat manager with atomic encrypted checkpoints, outbox retries, and Layer 234 transport integration.",
    ),
    entry(
        "BoundedSecretChatManager",
        "kiaro_secret_chat::runtime_modes::BoundedSecretChatManager",
        NativeItem,
        "Bounded custody manager with owner-bound CustodyStore, capacity-preserving pause, sweeps, and explicit safe resume.",
    ),
    entry(
        "EncryptedIngressStorage",
        "kiaro_secret_chat::ingress::EncryptedIngressStore",
        NativeItem,
        "AES-256-GCM encrypted ingress snapshot store with atomic replacement, empty-lockfile integrity, and stale-revision refusal.",
    ),
    entry(
        "IngressRefused",
        "kiaro_secret_chat::errors::SecretChatError::IngressRefused",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "DurableIngress",
        "kiaro_secret_chat::ingress::DurableIngressController",
        NativeItem,
        "Pre-acknowledgement capture, deduplication, contiguous applied cursor, bounded polling, and explicit durable acknowledgement.",
    ),
    entry(
        "StrongSecretChatManager",
        "kiaro_secret_chat::runtime_modes::StrongSecretChatManager",
        NativeItem,
        "Strong-ingress manager coupling persistent chat checkpoints with DurableIngressController contiguous cursor settlement.",
    ),
    entry(
        "IngressLimits",
        "kiaro_secret_chat::ingress::IngressLimits",
        PartialCapability,
        "Pending-record and payload bounds; not the baseline age, timeout and aggregate-limit configuration.",
    ),
    entry(
        "IngressBatch",
        "kiaro_secret_chat::ingress::DurableIngressRecord",
        PartialCapability,
        "One opaque record, not a complete difference-response batch and checkpoint.",
    ),
    entry(
        "CustodyLimits",
        "kiaro_secret_chat::custody::CustodyLimits",
        PartialCapability,
        "Payload count and byte limits; no manager-wide age, chat, task or deadline budgets.",
    ),
    entry(
        "ChatPaused",
        "",
        Unavailable,
        "A CustodyRecord pause flag exists; no named ChatPaused error or manager event exists.",
    ),
    entry(
        "PauseStatus",
        "",
        Unavailable,
        "No detached manager pause-status type or verified resumability assessment.",
    ),
    entry(
        "RelayDelivery",
        "",
        Unavailable,
        "Ingress record identities do not implement the baseline protocol-to-consumer relay obligation type.",
    ),
    entry(
        "TransientSecretChatManager",
        "kiaro_secret_chat::runtime_modes::TransientSecretChatManager",
        NativeItem,
        "RAM-only payload manager that permanently suspends active transient chats across restarts.",
    ),
    entry(
        "TransientLimits",
        "",
        Unavailable,
        "No transient manager task, deadline, retention-age or cleanup budget type.",
    ),
    entry(
        "TransientRefused",
        "",
        Unavailable,
        "No dedicated transient manager error or complete transient policy boundary.",
    ),
    entry(
        "TransientCleanupIncomplete",
        "",
        Unavailable,
        "No transient task-draining lifecycle or corresponding completion error.",
    ),
    entry(
        "ProtectedFileStorage",
        "kiaro_secret_chat::storage_backend::ProtectedFileStorage",
        NativeItem,
        "Account-bound encrypted file storage with atomic compare-and-swap checkpoints and strict deserialization.",
    ),
    entry(
        "SecretChat",
        "kiaro_secret_chat::protocol::SecretChat",
        NativeItem,
        "Live MTProto 2.0 protocol state with gap handling, PFS rekeying, TTL tracking, and lossless runtime checkpoints.",
    ),
    entry(
        "ChatSnapshot",
        "kiaro_secret_chat::storage_backend::PersistedChatRecord",
        NativeItem,
        "Account-bound persisted chat checkpoint record with redacted Debug output and zeroized key material.",
    ),
    entry(
        "ChatState",
        "kiaro_secret_chat::protocol::ChatState",
        NativeItem,
        "Rust protocol state enum; use the Rust transition methods and documented invariants.",
    ),
    entry(
        "StorageBackend",
        "kiaro_secret_chat::storage_backend::SecretChatStore",
        NativeItem,
        "Transactional secret-chat storage trait with compare-and-swap checkpoints and transient suspension.",
    ),
    entry(
        "MemoryStorage",
        "kiaro_secret_chat::storage_backend::MemoryStorage",
        NativeItem,
        "In-memory account-bound SecretChatStore implementation.",
    ),
    entry(
        "FileStorage",
        "kiaro_secret_chat::storage_backend::FileStorage",
        NativeItem,
        "Encrypted atomic file-backed SecretChatStore with empty-lockfile guard and duplicate-key rejection.",
    ),
    entry(
        "MEDIA_KINDS",
        "kiaro_secret_chat::public_api::MEDIA_KINDS",
        NativeItem,
        "All 8 secret-chat media kinds, paired with kiaro_secret_chat::media encrypted streaming, forwarding, and atomic save.",
    ),
    entry(
        "CAPTIONLESS_KINDS",
        "kiaro_secret_chat::public_api::CAPTIONLESS_KINDS",
        NativeItem,
        "Captionless media kinds enforced by kiaro_secret_chat::media::SecretMediaKind::allows_caption.",
    ),
    entry(
        "SecretChatError",
        "kiaro_secret_chat::errors::SecretChatError",
        NativeItem,
        "Rust error enum; variants and payloads are defined in errors.rs.",
    ),
    entry(
        "ParameterRejected",
        "kiaro_secret_chat::errors::SecretChatError::ParameterRejected",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "ChatNotReady",
        "kiaro_secret_chat::errors::SecretChatError::ChatNotReady",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "ChatClosed",
        "kiaro_secret_chat::errors::SecretChatError::ChatClosed",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "StorageRequired",
        "kiaro_secret_chat::errors::SecretChatError::StorageRequired",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "LayerUnsupported",
        "kiaro_secret_chat::errors::SecretChatError::LayerUnsupported",
        NativeItem,
        "Rust error enum variant; capability advertisement and wrapper-layer policy are separate.",
    ),
    entry(
        "ResendUnsatisfiable",
        "kiaro_secret_chat::errors::SecretChatError::ResendUnsatisfiable",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "StoreCorrupt",
        "kiaro_secret_chat::errors::SecretChatError::StoreCorrupt",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "UnknownChat",
        "kiaro_secret_chat::errors::SecretChatError::UnknownChat",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "ManagerStopping",
        "kiaro_secret_chat::errors::SecretChatError::ManagerStopping",
        NativeItem,
        "Rust error enum variant.",
    ),
    entry(
        "SendPending",
        "kiaro_secret_chat::errors::SecretChatError::SendPending",
        NativeItem,
        "An in-memory retained send may be retried; this variant does not promise restart recovery.",
    ),
    entry(
        "ChatRequested",
        "kiaro_secret_chat::transport::EncryptedChatWire::Requested",
        NativeItem,
        "Incoming secret-chat request wire representation accepted by establishment::SecretChatEstablisher::accept_chat.",
    ),
    entry(
        "ChatReady",
        "kiaro_secret_chat::manager::SecretEvent::ChatReady",
        NativeItem,
        "Emitted when a secret chat is established via SecretChatEstablisher or registered as ready.",
    ),
    entry(
        "ChatClosedEvent",
        "kiaro_secret_chat::manager::SecretEvent::ChatClosed",
        NativeItem,
        "Rust event enum variant, not a standalone baseline event class.",
    ),
    entry(
        "MessageReceived",
        "kiaro_secret_chat::manager::SecretEvent::MessageReceived",
        NativeItem,
        "Rust event enum variant; events remain in memory until the caller drains them.",
    ),
    entry(
        "MessageAcknowledged",
        "kiaro_secret_chat::manager::SecretEvent::MessageAcknowledged",
        NativeItem,
        "Authenticated peer acknowledgement event for retained sends; not an application settlement or read receipt.",
    ),
    entry(
        "ServiceActionReceived",
        "kiaro_secret_chat::manager::SecretEvent::ServiceActionReceived",
        NativeItem,
        "Rust event carrying an accepted service action; application TTL, deletion and read effects remain caller-owned.",
    ),
    entry(
        "DecryptFailed",
        "",
        Unavailable,
        "A returned MessageRejected error is not a DecryptFailed event.",
    ),
    entry(
        "SendFailed",
        "",
        Unavailable,
        "SendPending is not a permanent SendFailed event or the baseline self-delete replacement workflow.",
    ),
    entry(
        "MediaReference",
        "kiaro_secret_chat::media::ForwardMediaReference",
        NativeItem,
        "Validated encrypted media reference enabling zero-reupload forwarding via media::build_forward_media_descriptor.",
    ),
    entry(
        "KeyVisualization",
        "kiaro_secret_chat::establishment::KeyPicture",
        NativeItem,
        "Structured 12x12 key picture with 64-character hex and PNG rendering via render_key_visualization_png.",
    ),
    entry(
        "key_visualization",
        "kiaro_secret_chat::protocol::key_visualization",
        NativeItem,
        "Callable Rust function returning the documented JSON visualization representation.",
    ),
    entry(
        "PALETTE",
        "kiaro_secret_chat::public_api::PALETTE",
        NativeItem,
        "Shared visualization colors, verified against the implementation.",
    ),
];

pub const MEDIA_KINDS: [&str; 8] = [
    "photo",
    "video",
    "document",
    "audio",
    "animation",
    "sticker",
    "video_note",
    "voice_note",
];

pub const CAPTIONLESS_KINDS: [&str; 2] = ["sticker", "video_note"];

pub const PALETTE: [(u8, u8, u8); 4] = [
    (255, 255, 255),
    (213, 230, 243),
    (45, 87, 117),
    (47, 153, 201),
];
