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
        "kiaro_secret_chat::manager::SecretChatManager",
        PartialCapability,
        "In-memory manager for caller-registered established chats; no durable manager restoration or initial network establishment.",
    ),
    entry(
        "BoundedSecretChatManager",
        "",
        Unavailable,
        "CustodyRecord is a payload container, not a bounded protocol manager.",
    ),
    entry(
        "EncryptedIngressStorage",
        "kiaro_secret_chat::ingress::EncryptedIngressStore",
        PartialCapability,
        "Encrypted snapshot store; no complete protected checkpoint or protocol transaction integration.",
    ),
    entry(
        "IngressRefused",
        "kiaro_secret_chat::errors::SecretChatError::IngressRefused",
        NativeItem,
        "Rust error enum variant, not a Python exception class.",
    ),
    entry(
        "DurableIngress",
        "kiaro_secret_chat::ingress::DurableIngressController",
        PartialCapability,
        "Capture, deduplication, sorted polling and acknowledgement; no verified contiguous cursor or transport attachment.",
    ),
    entry(
        "StrongSecretChatManager",
        "",
        Unavailable,
        "No native manager binds transport acknowledgement, protocol state and consumer obligations in one durable lifecycle.",
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
        "",
        Unavailable,
        "A transient payload mode label is not a protected-state transient manager.",
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
        "",
        Unavailable,
        "CustodyStore is an encrypted payload store, not the baseline payload-free protocol projection.",
    ),
    entry(
        "SecretChat",
        "kiaro_secret_chat::protocol::SecretChat",
        PartialCapability,
        "Live protocol state, gap handling and rekey primitives; not a serializable manager restart record.",
    ),
    entry(
        "ChatSnapshot",
        "",
        Unavailable,
        "SecretChat is mutable live state, not a detached ChatSnapshot compatibility type.",
    ),
    entry(
        "ChatState",
        "kiaro_secret_chat::protocol::ChatState",
        NativeItem,
        "Rust protocol state enum; use the Rust transition methods and documented invariants.",
    ),
    entry(
        "StorageBackend",
        "",
        Unavailable,
        "CustodyBackend is a selection enum, not a transactional protocol storage trait.",
    ),
    entry(
        "MemoryStorage",
        "",
        Unavailable,
        "MemoryOnly is a custody selection, not a concrete protocol backend implementation.",
    ),
    entry(
        "FileStorage",
        "",
        Unavailable,
        "CustodyStore does not persist complete chat records, outboxes, gaps and manager settings.",
    ),
    entry(
        "MEDIA_KINDS",
        "kiaro_secret_chat::public_api::MEDIA_KINDS",
        NativeItem,
        "Media vocabulary only; does not implement encrypted upload, download or forwarding.",
    ),
    entry(
        "CAPTIONLESS_KINDS",
        "kiaro_secret_chat::public_api::CAPTIONLESS_KINDS",
        NativeItem,
        "Media vocabulary only; does not imply a native media sending API.",
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
        "",
        Unavailable,
        "No initial incoming-request transport handler or matching manager event.",
    ),
    entry(
        "ChatReady",
        "kiaro_secret_chat::manager::SecretEvent::ChatReady",
        PartialCapability,
        "Emitted for caller-registered established chats; initial network establishment is not implemented.",
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
        "",
        Unavailable,
        "AcceptedMessage is not a validated, serializable encrypted-file reference.",
    ),
    entry(
        "KeyVisualization",
        "",
        Unavailable,
        "The visualization function returns a JSON value; no named KeyVisualization Rust type is defined.",
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
