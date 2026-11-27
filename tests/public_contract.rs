use std::collections::BTreeSet;

use kiaro_secret_chat::public_api::{PUBLIC_EXPORTS, RustSurfaceStatus};

const BASELINE_EXPORTS: [&str; 48] = [
    "SecretChatManager",
    "BoundedSecretChatManager",
    "EncryptedIngressStorage",
    "IngressRefused",
    "DurableIngress",
    "StrongSecretChatManager",
    "IngressLimits",
    "IngressBatch",
    "CustodyLimits",
    "ChatPaused",
    "PauseStatus",
    "RelayDelivery",
    "TransientSecretChatManager",
    "TransientLimits",
    "TransientRefused",
    "TransientCleanupIncomplete",
    "ProtectedFileStorage",
    "SecretChat",
    "ChatSnapshot",
    "ChatState",
    "StorageBackend",
    "MemoryStorage",
    "FileStorage",
    "MEDIA_KINDS",
    "CAPTIONLESS_KINDS",
    "SecretChatError",
    "ParameterRejected",
    "ChatNotReady",
    "ChatClosed",
    "StorageRequired",
    "LayerUnsupported",
    "ResendUnsatisfiable",
    "StoreCorrupt",
    "UnknownChat",
    "ManagerStopping",
    "SendPending",
    "ChatRequested",
    "ChatReady",
    "ChatClosedEvent",
    "MessageReceived",
    "MessageAcknowledged",
    "ServiceActionReceived",
    "DecryptFailed",
    "SendFailed",
    "MediaReference",
    "KeyVisualization",
    "key_visualization",
    "PALETTE",
];

#[test]
fn baseline_name_inventory_is_complete_without_claiming_native_parity() {
    assert_eq!(PUBLIC_EXPORTS.len(), 48);
    let expected: BTreeSet<&str> = BASELINE_EXPORTS.into_iter().collect();
    let actual: BTreeSet<&str> = PUBLIC_EXPORTS.iter().map(|e| e.baseline_name).collect();
    assert_eq!(actual, expected);

    for entry in PUBLIC_EXPORTS {
        assert!(
            !entry.notes.is_empty(),
            "missing scope for {}",
            entry.baseline_name
        );
        match entry.status {
            RustSurfaceStatus::Unavailable => assert!(entry.native_path.is_empty()),
            RustSurfaceStatus::NativeItem | RustSurfaceStatus::PartialCapability => {
                assert!(entry.native_path.starts_with("kiaro_secret_chat::"));
            }
        }
    }
}

#[test]
fn missing_manager_storage_and_media_capabilities_are_explicit() {
    for name in [
        "BoundedSecretChatManager",
        "StrongSecretChatManager",
        "TransientSecretChatManager",
        "ProtectedFileStorage",
        "StorageBackend",
        "MemoryStorage",
        "FileStorage",
        "MediaReference",
        "ChatSnapshot",
        "RelayDelivery",
    ] {
        let entry = PUBLIC_EXPORTS
            .iter()
            .find(|entry| entry.baseline_name == name)
            .unwrap();
        assert_eq!(entry.status, RustSurfaceStatus::Unavailable, "{name}");
        assert!(
            entry.native_path.is_empty(),
            "{name} must not be mapped to an unrelated type"
        );
    }
    let manager = PUBLIC_EXPORTS
        .iter()
        .find(|entry| entry.baseline_name == "SecretChatManager")
        .unwrap();
    assert_eq!(manager.status, RustSurfaceStatus::PartialCapability);
}

#[test]
fn concrete_native_items_are_checked_by_the_compiler() {
    use kiaro_secret_chat::custody::{CustodyBackend, CustodyLimits, CustodyMode, CustodyRecord};
    use kiaro_secret_chat::errors::SecretChatError;
    use kiaro_secret_chat::ingress::{CommitReplayStatus, IngressLimits, IngressOrigin};
    use kiaro_secret_chat::protocol::{ChatRole, SecretChat, key_visualization};

    let record = CustodyRecord::new(
        7,
        "owner",
        CustodyMode::Bounded,
        CustodyBackend::EncryptedFile,
        CustodyLimits::default(),
    );
    let chat = SecretChat::new(7, 8, 9, "owner", ChatRole::Initiator);
    assert_eq!(record.chat_id(), chat.chat_id());
    assert!(IngressLimits::default().max_pending_records > 0);
    let _: Option<kiaro_secret_chat::custody::CustodyStore> = None;
    let _: Option<kiaro_secret_chat::ingress::EncryptedIngressStore> = None;
    let _: Option<kiaro_secret_chat::ingress::DurableIngressController> = None;
    let _: Option<kiaro_secret_chat::manager::SecretEvent> = None;
    let _: fn(&[u8]) -> Result<serde_json::Value, SecretChatError> = key_visualization;
    assert_ne!(
        CommitReplayStatus::Committed,
        CommitReplayStatus::DuplicateIgnored
    );
    assert_ne!(IngressOrigin::LiveSocket, IngressOrigin::DifferenceRecovery);
}
