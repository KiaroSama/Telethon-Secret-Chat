#![allow(clippy::unwrap_used)]
#[path = "support_compat/mod.rs"]
mod compat;
#[path = "support_compat/export_example.rs"]
mod export_example;
#[path = "support_compat/media_regressions.rs"]
mod media_regressions;
#[path = "support_compat/parser_regressions.rs"]
mod parser_regressions;
#[path = "support_compat/provenance.rs"]
mod provenance;
#[path = "support_sources/mod.rs"]
mod source_files;
#[path = "support_manager/mod.rs"]
mod support;
use grammers_tl_types as tl;
use kiaro_secret_chat::manager::{
    RpcFailure,
    transport::{PermanentError, classify_invocation_error},
};
use tl::{Deserializable, Identifiable};
#[test]
fn public_default_text_parser_exists() {
    parser_regressions::run();
    let parser: fn(&str) -> (String, Vec<tl::enums::MessageEntity>) =
        grammers_client::parsers::parse_markdown_message;
    let (text, entities) = parser("**bold** text");
    assert_eq!(text, "bold text");
    assert!(matches!(entities[0], tl::enums::MessageEntity::Bold(_)));
    let html: fn(&str) -> (String, Vec<tl::enums::MessageEntity>) =
        grammers_client::parsers::parse_html_message;
    assert_eq!(html("<b>bold</b>").0, "bold");
}
#[tokio::test]
async fn caller_entities_bypass_any_private_parser() {
    let (a, b, world, id) = support::established().await;
    a.send_message(id, "**bold**", Some(vec![]), None)
        .await
        .unwrap();
    support::pump(&a, &b, &world).await;
    let message = b.read_history(id, 1).unwrap().pop().unwrap();
    assert_eq!(message.text, "**bold**");
    assert!(message.entities.is_none());
}
#[test]
fn aes_ige_remains_public() {
    let encrypt: fn(&mut [u8], &[u8; 32], &[u8; 32]) = grammers_crypto::aes::ige_encrypt;
    let decrypt: fn(&mut [u8], &[u8; 32], &[u8; 32]) = grammers_crypto::aes::ige_decrypt;
    let plain = [9; 32];
    let mut bytes = plain;
    encrypt(&mut bytes, &[2; 32], &[3; 32]);
    assert_ne!(bytes, plain);
    decrypt(&mut bytes, &[2; 32], &[3; 32]);
    assert_eq!(bytes, plain);
}
#[test]
fn binary_cursor_and_primitive_decoder_remain_public() {
    let data = [4, 3, 2, 1, 3, b'a', b'b', b'c'];
    let mut cursor = tl::deserialize::Cursor::from_slice(&data);
    assert_eq!(i32::deserialize(&mut cursor).unwrap(), 0x01020304);
    assert_eq!(Vec::<u8>::deserialize(&mut cursor).unwrap(), b"abc");
    assert_eq!(cursor.pos(), data.len());
    assert!(cursor.read_byte().is_err());
}
#[test]
fn all_referenced_tl_requests_and_types_compile_on_one_graph() {
    media_regressions::run();
    let _ = export_example::example;
    let snippet =
        include_str!("support_compat/export_example.rs").replace("pub fn example", "fn example");
    assert!(include_str!("../docs/rust-export.md").contains(snippet.trim()));
    use std::collections::BTreeSet;
    struct Paths(BTreeSet<String>);
    impl<'a> syn::visit::Visit<'a> for Paths {
        fn visit_path(&mut self, path: &'a syn::Path) {
            let parts = path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>();
            if parts.len() > 1 && parts[0] == "tl" {
                self.0.insert(parts[1..].join("::"));
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut paths = Paths(BTreeSet::new());
    for file in compat::source_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
    {
        let tree = syn::parse_file(&std::fs::read_to_string(file).unwrap()).unwrap();
        syn::visit::Visit::visit_file(&mut paths, &tree);
    }
    for required in [
        "functions::messages::SendEncrypted",
        "functions::messages::SendEncryptedService",
        "functions::messages::SendEncryptedFile",
        "enums::Update::NewEncryptedMessage",
        "enums::EncryptedChat::Discarded",
    ] {
        assert!(
            paths.0.contains(required),
            "SDK surface discovery lost {required}"
        );
    }
    fn request<T: tl::RemoteCall>() {}
    request::<tl::functions::messages::SendEncrypted>();
    request::<tl::functions::messages::SendEncryptedService>();
    request::<tl::functions::messages::SendEncryptedFile>();
    assert_eq!(
        tl::name_for_id(tl::types::UpdateNewEncryptedMessage::CONSTRUCTOR_ID),
        "updateNewEncryptedMessage"
    );
    assert_eq!(
        tl::name_for_id(tl::types::EncryptedChatDiscarded::CONSTRUCTOR_ID),
        "encryptedChatDiscarded"
    );
    let same: for<'a> fn(
        &'a kiaro_secret_chat::grammers_client::Client,
    ) -> &'a grammers_client::Client = |client| client;
    let _ = same;
    for name in [
        "grammers-client",
        "grammers-session",
        "grammers-mtsender",
        "grammers-tl-types",
    ] {
        compat::package(name);
    }
}
#[test]
fn independent_kdf_oracle_still_exists() {
    let source = compat::dependency_source("grammers-crypto");
    let tree = syn::parse_file(&source).unwrap();
    let function = tree
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(f) if f.sig.ident == "calc_key" => Some(f),
            _ => None,
        })
        .unwrap();
    let names = function
        .sig
        .inputs
        .iter()
        .map(|arg| match arg {
            syn::FnArg::Typed(a) => match &*a.pat {
                syn::Pat::Ident(p) => p.ident.to_string(),
                _ => panic!("unexpected independent KDF argument"),
            },
            _ => panic!("unexpected oracle receiver"),
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["auth_key", "msg_key", "side"]);
    let encrypt: fn(&mut [u8], &grammers_crypto::AuthKey) -> [u8; 16] =
        grammers_crypto::encrypt_data_v2;
    let decrypt: for<'a> fn(
        &'a mut [u8],
        &grammers_crypto::AuthKey,
    ) -> Result<&'a mut [u8], grammers_crypto::Error> = grammers_crypto::decrypt_data_v2;
    let _ = (encrypt, decrypt);
}
#[test]
fn supported_grammers_version_and_revision_are_stated() {
    provenance::verify();
    let root = compat::package("kiaro-secret-chat");
    assert_eq!(root["version"], kiaro_secret_chat::VERSION);
    for name in [
        "grammers-client",
        "grammers-session",
        "grammers-mtsender",
        "grammers-crypto",
        "grammers-tl-types",
    ] {
        let package = compat::package(name);
        assert_eq!(package["version"], "0.12.0");
        let dep = root["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap();
        assert_eq!(dep["req"], "=0.12.0");
    }
    for name in ["grammers-crypto", "grammers-tl-types"] {
        assert!(
            compat::package(name)["source"]
                .as_str()
                .unwrap()
                .contains("e61cf8585e1e9969ab81eda9fb4725a1500e2d86")
        );
    }
    assert_eq!(compat::package("grammers-tl-parser")["version"], "2.0.0");
}
fn rpc(name: &str) -> grammers_client::InvocationError {
    grammers_client::InvocationError::Rpc(grammers_mtsender::RpcError::from(tl::types::RpcError {
        error_code: 400,
        error_message: name.into(),
    }))
}
#[test]
fn acceptance_error_names_have_native_classifications() {
    for (name, expected) in [
        ("ENCRYPTION_ALREADY_ACCEPTED", RpcFailure::AlreadyAccepted),
        ("ENCRYPTION_ALREADY_DECLINED", RpcFailure::Declined),
        ("ENCRYPTION_DECLINED", RpcFailure::ChatEnded),
        ("ENCRYPTION_ID_INVALID", RpcFailure::ChatEnded),
    ] {
        assert_eq!(classify_invocation_error(rpc(name)), expected);
    }
}
#[test]
fn send_rejections_have_native_classifications() {
    for name in [
        "CHAT_ID_INVALID",
        "ENCRYPTION_DECLINED",
        "ENCRYPTION_ID_INVALID",
        "USER_DELETED",
    ] {
        assert_eq!(classify_invocation_error(rpc(name)), RpcFailure::ChatEnded);
    }
    for (name, expected) in [
        ("DATA_INVALID", PermanentError::DataInvalid),
        ("DATA_TOO_LONG", PermanentError::DataTooLong),
        ("FILE_EMPTY", PermanentError::FileEmpty),
        ("MD5_CHECKSUM_INVALID", PermanentError::Md5ChecksumInvalid),
    ] {
        assert_eq!(
            classify_invocation_error(rpc(name)),
            RpcFailure::Rejected(expected)
        );
        assert!(expected.name().ends_with("Error"));
    }
    let private = "PUBLIC-BURNED-ERROR-MARKER";
    let failure = classify_invocation_error(rpc(private));
    assert_eq!(failure, RpcFailure::Transient);
    assert!(!format!("{failure:?}").contains(private));
}
