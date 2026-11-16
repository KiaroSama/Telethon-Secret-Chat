#![allow(clippy::unwrap_used)]
#[path = "support_manager/compile_api.rs"]
mod compile_boundary;
#[path = "support_manager/seeded.rs"]
mod seeded;
#[path = "support_manager/mod.rs"]
mod support;
use compile_boundary::compile_api;
use kiaro_secret_chat::{
    error::Error,
    manager::{Options, SecretChatManager},
    storage::MemoryStorage,
};
use seeded::*;
use std::{collections::BTreeSet, path::Path, sync::Arc};
use support::*;
fn valid_api() {
    let output = compile_api(
        "use kiaro_secret_chat::{manager::{Options,SecretChatManager},transport::GrammersTransport,storage::MemoryStorage};fn valid(client:GrammersTransport){let _=SecretChatManager::new(client,MemoryStorage::default(),\"11\".repeat(32),Options::default());}",
    );
    assert!(
        output.status.success(),
        "valid managed API: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn names(tree: &syn::UseTree, set: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(path) => names(&path.tree, set),
        syn::UseTree::Name(name) => {
            set.insert(name.ident.to_string());
        }
        syn::UseTree::Rename(name) => {
            set.insert(name.rename.to_string());
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                names(tree, set);
            }
        }
        syn::UseTree::Glob(_) => panic!("public managed exports must be explicit"),
    }
}
#[test]
fn managed_module_exports_exact_native_contract() {
    let file = syn::parse_file(include_str!("../src/manager/mod.rs")).unwrap();
    let mut exports = BTreeSet::new();
    for item in file.items {
        match item {
            syn::Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                names(&item.tree, &mut exports)
            }
            syn::Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                exports.insert(item.ident.to_string());
            }
            syn::Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                exports.insert(item.ident.to_string());
            }
            syn::Item::Mod(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                exports.insert(item.ident.to_string());
            }
            _ => {}
        }
    }
    let expected = [
        "Admission",
        "BoundedSecretChatManager",
        "Event",
        "MessageReceived",
        "Options",
        "PermanentError",
        "RpcFailure",
        "SecretChatManager",
        "SendFileOptions",
        "Sent",
        "StrongSecretChatManager",
        "TransientSecretChatManager",
        "Transport",
        "transport",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(exports, expected);
    valid_api();
}
#[test]
fn low_level_key_material_has_no_hidden_root_alias() {
    valid_api();
    let output = compile_api("use kiaro_secret_chat::{SharedKey,SecretExponent,Protocol};");
    assert!(!output.status.success());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("unresolved imports"));
    assert!(
        diagnostic.contains("SharedKey")
            && diagnostic.contains("SecretExponent")
            && diagnostic.contains("Protocol")
    );
}
#[test]
fn managed_establishment_accepts_no_key_or_skip_override() {
    valid_api();
    let file = syn::parse_file(include_str!("../src/manager/establishment.rs")).unwrap();
    let module = syn::parse_file(include_str!("../src/manager/mod.rs")).unwrap();
    let conversation = syn::parse_file(include_str!("../src/manager/conversation.rs")).unwrap();
    for item in file
        .items
        .into_iter()
        .chain(module.items)
        .chain(conversation.items)
    {
        if let syn::Item::Impl(item) = item {
            for member in item.items {
                if let syn::ImplItem::Fn(method) = member
                    && ["new", "verified", "create", "accept", "send_message"]
                        .iter()
                        .any(|name| method.sig.ident == name)
                {
                    for argument in method.sig.inputs {
                        if let syn::FnArg::Typed(argument) = argument
                            && let syn::Pat::Ident(argument) = *argument.pat
                        {
                            let name = argument.ident.to_string();
                            assert!(
                                ![
                                    "key", "skip", "trust", "unsafe", "insecure", "force", "layer"
                                ]
                                .iter()
                                .any(|part| name.contains(part))
                            );
                        }
                    }
                }
            }
        }
    }
    let output = compile_api(
        "use kiaro_secret_chat::{manager::{Options,SecretChatManager},transport::GrammersTransport,storage::MemoryStorage,crypto::SharedKey};fn invalid(client:GrammersTransport,key:SharedKey){let _=SecretChatManager::new(client,MemoryStorage::default(),key,Options::default());}",
    );
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("E0308"));
}
#[tokio::test]
async fn unknown_chat_is_structured_lookup_and_package_error() {
    let (a, _store, _w, _id) = ready(Options::default()).await;
    let error = a.status(999999).unwrap_err();
    assert!(matches!(error, Error::UnknownChat { chat_id: 999999 }));
    let family: &(dyn std::error::Error + Send + Sync) = &error;
    assert!(!family.to_string().is_empty());
    assert!(matches!(
        a.read_history(999999, 1),
        Err(Error::UnknownChat { chat_id: 999999 })
    ));
    a.stop().await.unwrap();
}
#[tokio::test]
async fn snapshots_are_detached_metadata_without_key_material() {
    let (a, _store, _w, id) = ready(Options::default()).await;
    let original = a.status(id).unwrap();
    let mut copy = original.clone();
    copy.ttl = 99;
    copy.key_fingerprint = None;
    copy.peer_user_id = 12345;
    assert_eq!(a.status(id).unwrap(), original);
    assert_eq!(a.list().unwrap(), vec![original]);
    valid_api();
    for field in ["key", "previous_key", "exchange_secret", "handshake_secret"] {
        let code = format!(
            "use kiaro_secret_chat::chat::ChatSnapshot;fn invalid(snapshot:ChatSnapshot){{let _=snapshot.{field};}}"
        );
        let output = compile_api(&code);
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr).unwrap().contains("E0609"));
    }
    a.stop().await.unwrap();
}
#[test]
fn every_public_manager_method_has_rust_documentation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/manager");
    let mut count = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let file = syn::parse_file(&std::fs::read_to_string(path).unwrap()).unwrap();
        for item in file.items {
            if let syn::Item::Impl(item) = item {
                if !matches!(&*item.self_ty,syn::Type::Path(path) if path.path.segments.last().is_some_and(|segment|segment.ident=="SecretChatManager"||segment.ident=="StrongSecretChatManager"))
                {
                    continue;
                }
                for member in item.items {
                    if let syn::ImplItem::Fn(method) = member
                        && matches!(method.vis, syn::Visibility::Public(_))
                    {
                        count += 1;
                        assert!(
                            method
                                .attrs
                                .iter()
                                .any(|attribute| attribute.path().is_ident("doc")),
                            "undocumented public method {}",
                            method.sig.ident
                        );
                    }
                }
            }
        }
    }
    assert!(count >= 40);
}
fn history_type(value: &str) {
    valid_api();
    let source = format!(
        "use kiaro_secret_chat::{{manager::SecretChatManager,transport::GrammersTransport,storage::MemoryStorage}};fn invalid(manager:SecretChatManager<MemoryStorage,GrammersTransport>){{let _=manager.read_history(7,{value});}}"
    );
    let output = compile_api(&source);
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("usize"));
}
#[test]
fn history_refuses_negative_count_at_rust_boundary() {
    history_type("-1");
}
#[test]
fn history_refuses_string_count_at_rust_boundary() {
    history_type("\"5\"");
}
#[test]
fn history_refuses_float_count_at_rust_boundary() {
    history_type("2.0");
}
#[test]
fn explicit_backend_is_required_by_native_constructor() {
    valid_api();
    let output = compile_api(
        "use kiaro_secret_chat::{manager::{Options,SecretChatManager},transport::GrammersTransport};fn invalid(client:GrammersTransport){let _=SecretChatManager::new(client,None,\"11\".repeat(32),Options::default());}",
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Storage")
    );
}
#[tokio::test]
async fn manager_preserves_original_client_fields_and_identity() {
    let (client, _) = Fake::pair();
    let original = client.clone();
    let world = Arc::clone(&client.world);
    let manager = SecretChatManager::new(
        client,
        MemoryStorage::default(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    manager.start().await.unwrap();
    manager.stop().await.unwrap();
    assert_eq!(original.user, 1000);
    assert!(Arc::ptr_eq(&original.world, &world));
    let world = world.lock().unwrap();
    assert!(
        world.requests.is_empty()
            && world.sent.is_empty()
            && world.uploads == 0
            && world.downloads == 0
    );
}
#[test]
fn numeric_media_source_is_refused_by_native_type() {
    valid_api();
    let output = compile_api(
        "use kiaro_secret_chat::{manager::{SecretChatManager,SendFileOptions},transport::GrammersTransport,storage::MemoryStorage};async fn invalid(manager:SecretChatManager<MemoryStorage,GrammersTransport>){let _=manager.send_file(7,12345,SendFileOptions::default()).await;}",
    );
    assert!(!output.status.success());
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert!(diagnostic.contains("E0308") && diagnostic.contains("FileSource"));
}
#[test]
fn authored_rust_sources_preserve_eight_hundred_line_ceiling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pending = vec![root.join("src"), root.join("tests"), root.join("tools")];
    let mut over = Vec::new();
    while let Some(folder) = pending.pop() {
        let entries = match std::fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => panic!("source scan failed: {error}"),
        };
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let content = std::fs::read_to_string(&path).unwrap();
                let generated = content
                    .lines()
                    .next()
                    .is_some_and(|first| first.starts_with("// Generated from schema/"));
                if !generated && content.lines().count() > 800 {
                    over.push(path.strip_prefix(root).unwrap().display().to_string());
                }
            }
        }
    }
    assert!(over.is_empty(), "overlong Rust sources: {over:?}");
}
