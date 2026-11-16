#![allow(clippy::unwrap_used)]
use super::{manager_support, refusals};
use kiaro_secret_chat::{error::Error, media::MediaKind};
use std::{collections::BTreeSet, fs, path::Path};
const FAMILIES: [&str; 11] = [
    "ParameterRejected",
    "MessageRejected",
    "ChatNotReady",
    "ChatClosed",
    "StorageRequired",
    "LayerUnsupported",
    "ResendUnsatisfiable",
    "StoreCorrupt",
    "UnknownChat",
    "ManagerStopping",
    "SendPending",
];
fn probes() -> Vec<(&'static str, Error, bool)> {
    vec![
        (
            "DhFailure",
            Error::DhFailure(kiaro_secret_chat::error::DhFailure::Length),
            false,
        ),
        (
            "ChatNotSendable",
            Error::ChatNotSendable {
                chat_id: 42,
                state: kiaro_secret_chat::chat::ChatState::Requested,
            },
            false,
        ),
        (
            "ChatEnded",
            Error::ChatEnded {
                chat_id: 42,
                reason: kiaro_secret_chat::error::CloseReason::PeerDiscarded,
            },
            false,
        ),
        ("Context", Error::FrameRejected.with_chat(42), false),
        (
            "MediaField",
            Error::MediaField {
                field: refusals::PRIVATE_TEXT,
            },
            true,
        ),
        (
            "MediaKind",
            Error::MediaKind {
                kind: refusals::PRIVATE_TEXT.into(),
            },
            true,
        ),
        (
            "MediaKindMismatch",
            Error::MediaKindMismatch {
                file_name: refusals::PRIVATE_TEXT.into(),
                kind: "video",
            },
            true,
        ),
        (
            "MediaCaption",
            Error::MediaCaption {
                kind: refusals::PRIVATE_TEXT,
            },
            true,
        ),
        (
            "SendPending",
            Error::SendPending {
                chat_id: 42,
                random_id: 99,
                cause: kiaro_secret_chat::error::PendingCause::Transport,
            },
            false,
        ),
        ("ManagerStopping", Error::ManagerStopping, false),
        ("UnknownChat", Error::UnknownChat { chat_id: 42 }, false),
        ("ChatClosed", Error::ChatClosed { chat_id: 42 }, false),
        ("ChatNotReady", Error::ChatNotReady { chat_id: 42 }, false),
        ("Randomness", Error::Randomness, false),
        ("KeyLength", Error::KeyLength, false),
        ("FrameRejected", Error::FrameRejected, false),
        ("WrapperRejected", Error::WrapperRejected, false),
        ("InnerMessageRejected", Error::InnerMessageRejected, false),
        ("DhParameters", Error::DhParameters, false),
        ("PeerValue", Error::PeerValue, false),
        ("Fingerprint", Error::Fingerprint, false),
        ("Sequence", Error::Sequence, false),
        ("ResendUnavailable", Error::ResendUnavailable, false),
        ("ChatState", Error::ChatState, false),
        ("NotRunning", Error::NotRunning, false),
        ("Paused", Error::Paused, false),
        ("Capacity", Error::Capacity, false),
        ("Storage", Error::Storage, false),
        ("InvalidState", Error::InvalidState, false),
        ("Media", Error::Media, false),
        ("Transport", Error::Transport, false),
        ("Authorization", Error::Authorization, false),
        ("IngressUnavailable", Error::IngressUnavailable, false),
    ]
}
fn storage_is_explicit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let protocol =
        syn::parse_file(&fs::read_to_string(root.join("src/protocol.rs")).unwrap()).unwrap();
    let mut found = false;
    for item in protocol.items {
        if let syn::Item::Impl(i) = item {
            let bound=i.generics.params.iter().any(|p|matches!(p,syn::GenericParam::Type(t)if t.ident=="S"&&t.bounds.iter().any(|b|matches!(b,syn::TypeParamBound::Trait(t)if t.path.segments.last().is_some_and(|s|s.ident=="Storage")))));
            for member in i.items {
                if let syn::ImplItem::Fn(f) = member
                    && f.sig.ident == "open"
                {
                    assert!(bound, "Protocol type must require Storage");
                    let first = f.sig.inputs.first().unwrap();
                    assert!(
                        matches!(first,syn::FnArg::Typed(a)if matches!(a.ty.as_ref(),syn::Type::Path(p)if p.path.is_ident("S"))),
                        "an optional/missing backend cannot satisfy the typed constructor"
                    );
                    assert_eq!(f.sig.inputs.len(), 2);
                    found = true;
                }
            }
        }
    }
    assert!(found, "audited explicit-storage constructor disappeared");
}
pub fn protocol_object_refusal() {
    let object = refusals::message();
    let bytes = kiaro_secret_chat::Serializable::to_bytes(&object);
    assert!(
        bytes
            .windows(refusals::PLAIN.len())
            .any(|b| b == refusals::PLAIN.as_bytes())
    );
    let error = refusals::catch(kiaro_secret_chat::framing::unwrap(&bytes));
    refusals::assert_private(&error);
    assert!(matches!(error, Error::Context(_)));
    let kind = MediaKind::parse(refusals::PRIVATE_TEXT).unwrap_err();
    assert!(
        kind.to_string().contains(refusals::PRIVATE_TEXT),
        "ordinary caller input validation keeps supplied metadata useful"
    );
    let protected = kind.with_chat(42);
    refusals::assert_private(&protected);
    assert!(matches!(protected,Error::Context(ref c)if matches!(c.refusal(),Error::Media)));
    protected_metadata_refusal();
}
fn protected_metadata_refusal() {
    use kiaro_secret_chat::{
        custody::{TransientLimits, TransientStorage},
        manager::{Options, SecretChatManager, SendFileOptions},
        media::FileSource,
        storage::MemoryStorage,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (aa, bb) = manager_support::Fake::pair();
        let world = aa.world.clone();
        let a = SecretChatManager::new(
            aa,
            TransientStorage::new(
                MemoryStorage::default(),
                "11".repeat(32),
                TransientLimits::default(),
            )
            .unwrap(),
            "11".repeat(32),
            Options::default(),
        )
        .unwrap();
        let b = SecretChatManager::new(
            bb,
            MemoryStorage::default(),
            "22".repeat(32),
            Options::default(),
        )
        .unwrap();
        a.start().await.unwrap();
        b.start().await.unwrap();
        let id = a.create(manager_support::peer(2000)).await.unwrap().id;
        manager_support::pump(&a, &b, &world).await;
        b.accept(id).await.unwrap();
        manager_support::pump(&a, &b, &world).await;
        assert_eq!(
            a.status(id).unwrap().state,
            kiaro_secret_chat::chat::ChatState::Ready
        );
        let canary = format!("{}-private-name.jpg", refusals::PRIVATE_TEXT);
        let result = a
            .send_file(
                id,
                FileSource::bytes(b"burned image bytes".to_vec(), canary.clone()).unwrap(),
                SendFileOptions {
                    kind: Some(MediaKind::Video),
                    mime_type: Some("image/jpeg".into()),
                    ..Default::default()
                },
            )
            .await;
        let error = refusals::catch(result);
        for rendered in refusals::render(&error) {
            assert!(!rendered.contains(&canary));
        }
        assert!(matches!(error,Error::Context(ref c)if matches!(c.refusal(),Error::Paused)));
        assert_eq!(world.lock().unwrap().uploads, 0);
        a.stop().await.unwrap();
        b.stop().await.unwrap();
    });
}
pub fn all_errors_have_no_opaque_run() {
    for name in FAMILIES {
        let error = refusals::family(name);
        refusals::assert_no_opaque(&error, false);
    }
    for (_, error, caller_metadata) in probes() {
        let error = if caller_metadata {
            error.with_chat(42)
        } else {
            error
        };
        refusals::assert_no_opaque(&error, false);
    }
}
pub fn one_native_error_family() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/error.rs")).unwrap();
    let tree = syn::parse_file(&source).unwrap();
    let declared = tree
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == "Error" => Some(
                e.variants
                    .iter()
                    .map(|v| v.ident.to_string())
                    .collect::<BTreeSet<_>>(),
            ),
            _ => None,
        })
        .unwrap();
    let probes = probes();
    let covered = probes
        .iter()
        .map(|(name, _, _)| name.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared, covered,
        "every newly exported native error variant needs a privacy classification"
    );
    for (_, error, caller_metadata) in probes {
        let _: &dyn std::error::Error = &error;
        if caller_metadata {
            assert!(error.to_string().contains(refusals::PRIVATE_TEXT));
            refusals::assert_private(&error.with_chat(42));
        } else {
            refusals::assert_private(&error);
        }
    }
    for name in FAMILIES {
        let error = refusals::family(name);
        let _: &dyn std::error::Error = &error;
        refusals::assert_private(&error);
    }
    storage_is_explicit();
}
