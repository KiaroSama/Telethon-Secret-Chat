use std::collections::BTreeSet;

use kiaro_secret_chat::public_api::PUBLIC_EXPORTS;

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
fn public_contract_resolves_all_48_baseline_exports_with_zero_python_fallback() {
    assert_eq!(PUBLIC_EXPORTS.len(), 48);
    let expected: BTreeSet<&str> = BASELINE_EXPORTS.into_iter().collect();
    let actual: BTreeSet<&str> = PUBLIC_EXPORTS.iter().map(|e| e.baseline_name).collect();
    assert_eq!(actual, expected);

    for entry in PUBLIC_EXPORTS {
        assert!(
            !entry.native_path.is_empty(),
            "missing native path for {}",
            entry.baseline_name
        );
        assert!(
            !entry.native_path.contains("python") && !entry.native_path.contains("pyo3"),
            "export {} must not rely on Python/PyO3: {}",
            entry.baseline_name,
            entry.native_path
        );
    }
}
