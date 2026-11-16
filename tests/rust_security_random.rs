#![allow(clippy::unwrap_used)]
#[path = "support_security/audit.rs"]
mod audit;
#[path = "support_security/producers.rs"]
mod producers;
#[test]
fn there_are_native_sources_to_check() {
    let sources = audit::sources();
    assert!(sources.len() > 100);
    assert!(sources.contains(&audit::root().join("tools/secret-scan/src/main.rs")));
    assert_eq!(audit::scopes().len(), 88);
    for path in sources {
        let tree = audit::parse(&path);
        assert!(
            audit::audit(&tree).is_empty(),
            "new or retained native source {} reaches insecure randomness: {:?}",
            path.display(),
            audit::audit(&tree)
        );
    }
    let good = syn::parse_file(
        r#"// rand::random() in a comment is not executable.
const DOC:&str="rand::rng() random";
fn random_id()->u64{getrandom::u64().unwrap()}"#,
    )
    .unwrap();
    assert!(audit::audit(&good).is_empty());
    for bad in [
        "use rand;",
        "use rand::{rng as alias};",
        "extern crate rand as alias;",
        "fn key(){let _=rand::random::<u64>();}",
        "macro_rules! key {() => {fastrand::u64(..)}}",
        "macro_rules! key {() => {use rand as weak;}}",
    ] {
        assert!(
            !audit::audit(&syn::parse_file(bad).unwrap()).is_empty(),
            "AST auditor missed {bad}"
        );
    }
}
#[test]
fn generated_schema_is_audited() {
    let sources = audit::sources();
    let generated = ["secret_types.rs", "secret_enums.rs", "secret_common.rs"]
        .map(|name| std::path::PathBuf::from(env!("OUT_DIR")).join(name));
    for path in generated {
        assert!(sources.contains(&path));
        let tree = audit::parse(&path);
        assert!(!tree.items.is_empty());
        assert!(audit::audit(&tree).is_empty());
    }
    audit::check_scope("secret_tl.py");
}
#[test]
fn secure_sources_are_actually_used() {
    producers::check();
}
#[test]
fn module_init_py0() {
    audit::check_scope("__init__.py0");
}
#[test]
fn module_actions_py() {
    audit::check_scope("actions.py");
}
#[test]
fn module_autosave_py() {
    audit::check_scope("autosave.py");
}
#[test]
fn module_bounded_py() {
    audit::check_scope("bounded.py");
}
#[test]
fn module_bounded_media_py() {
    audit::check_scope("bounded_media.py");
}
#[test]
fn module_bounded_storage_py() {
    audit::check_scope("bounded_storage.py");
}
#[test]
fn module_bounded_transport_py() {
    audit::check_scope("bounded_transport.py");
}
#[test]
fn module_bounded_types_py() {
    audit::check_scope("bounded_types.py");
}
#[test]
fn module_chat_py() {
    audit::check_scope("chat.py");
}
#[test]
fn module_crypto_py() {
    audit::check_scope("crypto.py");
}
#[test]
fn module_dh_py() {
    audit::check_scope("dh.py");
}
#[test]
fn module_dispatch_py() {
    audit::check_scope("dispatch.py");
}
#[test]
fn module_entities_py() {
    audit::check_scope("entities.py");
}
#[test]
fn module_errors_py() {
    audit::check_scope("errors.py");
}
#[test]
fn module_establishment_py() {
    audit::check_scope("establishment.py");
}
#[test]
fn module_events_py() {
    audit::check_scope("events.py");
}
#[test]
fn module_files_py0() {
    audit::check_scope("files.py0");
}
#[test]
fn module_framing_py() {
    audit::check_scope("framing.py");
}
#[test]
fn module_handshake_py() {
    audit::check_scope("handshake.py");
}
#[test]
fn module_host_py() {
    audit::check_scope("host.py");
}
#[test]
fn module_ingress_py() {
    audit::check_scope("ingress.py");
}
#[test]
fn module_ingress_machine_py() {
    audit::check_scope("ingress_machine.py");
}
#[test]
fn module_ingress_provenance_py() {
    audit::check_scope("ingress_provenance.py");
}
#[test]
fn module_ingress_replay_py() {
    audit::check_scope("ingress_replay.py");
}
#[test]
fn module_ingress_requests_py() {
    audit::check_scope("ingress_requests.py");
}
#[test]
fn module_ingress_store_py() {
    audit::check_scope("ingress_store.py");
}
#[test]
fn module_ingress_types_py() {
    audit::check_scope("ingress_types.py");
}
#[test]
fn module_ingress_validation_py() {
    audit::check_scope("ingress_validation.py");
}
#[test]
fn module_locking_py() {
    audit::check_scope("locking.py");
}
#[test]
fn module_manager_py() {
    audit::check_scope("manager.py");
}
#[test]
fn module_media_py() {
    audit::check_scope("media.py");
}
#[test]
fn module_ogg_tags_py() {
    audit::check_scope("ogg_tags.py");
}
#[test]
fn module_outbox_py() {
    audit::check_scope("outbox.py");
}
#[test]
fn module_protected_py() {
    audit::check_scope("protected.py");
}
#[test]
fn module_receive_py() {
    audit::check_scope("receive.py");
}
#[test]
fn module_rekey_py() {
    audit::check_scope("rekey.py");
}
#[test]
fn module_saved_export_py() {
    audit::check_scope("saved_export.py");
}
#[test]
fn module_init_py1() {
    audit::check_scope("__init__.py1");
}
#[test]
fn module_secret_tl_py() {
    audit::check_scope("secret_tl.py");
}
#[test]
fn module_sequence_py() {
    audit::check_scope("sequence.py");
}
#[test]
fn module_init_py2() {
    audit::check_scope("__init__.py2");
}
#[test]
fn module_strong_manager_py() {
    audit::check_scope("strong_manager.py");
}
#[test]
fn module_init_py3() {
    audit::check_scope("__init__.py3");
}
#[test]
fn module_fetch_py() {
    audit::check_scope("fetch.py");
}
#[test]
fn module_fetch_files_py() {
    audit::check_scope("fetch_files.py");
}
#[test]
fn module_files_py1() {
    audit::check_scope("files.py1");
}
#[test]
fn module_html_and_json_py() {
    audit::check_scope("html_and_json.py");
}
#[test]
fn module_html_countries_py() {
    audit::check_scope("html_countries.py");
}
#[test]
fn module_html_layout_py() {
    audit::check_scope("html_layout.py");
}
#[test]
fn module_html_media_py() {
    audit::check_scope("html_media.py");
}
#[test]
fn module_html_media_extra_py() {
    audit::check_scope("html_media_extra.py");
}
#[test]
fn module_html_message_py() {
    audit::check_scope("html_message.py");
}
#[test]
fn module_html_qurl_py() {
    audit::check_scope("html_qurl.py");
}
#[test]
fn module_html_rich_py() {
    audit::check_scope("html_rich.py");
}
#[test]
fn module_html_rich_media_py() {
    audit::check_scope("html_rich_media.py");
}
#[test]
fn module_html_rich_support_py() {
    audit::check_scope("html_rich_support.py");
}
#[test]
fn module_html_rich_text_py() {
    audit::check_scope("html_rich_text.py");
}
#[test]
fn module_html_safe_py() {
    audit::check_scope("html_safe.py");
}
#[test]
fn module_html_service_py() {
    audit::check_scope("html_service.py");
}
#[test]
fn module_html_text_py() {
    audit::check_scope("html_text.py");
}
#[test]
fn module_html_writer_py() {
    audit::check_scope("html_writer.py");
}
#[test]
fn module_json_actions_py() {
    audit::check_scope("json_actions.py");
}
#[test]
fn module_json_message_py() {
    audit::check_scope("json_message.py");
}
#[test]
fn module_json_rich_py() {
    audit::check_scope("json_rich.py");
}
#[test]
fn module_json_serialize_py() {
    audit::check_scope("json_serialize.py");
}
#[test]
fn module_json_writer_py() {
    audit::check_scope("json_writer.py");
}
#[test]
fn module_mime_globs_py() {
    audit::check_scope("mime_globs.py");
}
#[test]
fn module_model_py() {
    audit::check_scope("model.py");
}
#[test]
fn module_model_actions_py() {
    audit::check_scope("model_actions.py");
}
#[test]
fn module_model_dialogs_py() {
    audit::check_scope("model_dialogs.py");
}
#[test]
fn module_model_format_py() {
    audit::check_scope("model_format.py");
}
#[test]
fn module_model_media_py() {
    audit::check_scope("model_media.py");
}
#[test]
fn module_model_message_py() {
    audit::check_scope("model_message.py");
}
#[test]
fn module_model_peers_py() {
    audit::check_scope("model_peers.py");
}
#[test]
fn module_model_rich_py() {
    audit::check_scope("model_rich.py");
}
#[test]
fn module_model_rich_parse_py() {
    audit::check_scope("model_rich_parse.py");
}
#[test]
fn module_output_file_py() {
    audit::check_scope("output_file.py");
}
#[test]
fn module_qt_icc_py() {
    audit::check_scope("qt_icc.py");
}
#[test]
fn module_qt_jpeg_py() {
    audit::check_scope("qt_jpeg.py");
}
#[test]
fn module_qt_scale_py() {
    audit::check_scope("qt_scale.py");
}
#[test]
fn module_secret_saved_py() {
    audit::check_scope("secret_saved.py");
}
#[test]
fn module_settings_py() {
    audit::check_scope("settings.py");
}
#[test]
fn module_temporary_py() {
    audit::check_scope("temporary.py");
}
#[test]
fn module_transient_py() {
    audit::check_scope("transient.py");
}
#[test]
fn module_transient_media_py() {
    audit::check_scope("transient_media.py");
}
#[test]
fn module_transient_storage_py() {
    audit::check_scope("transient_storage.py");
}
#[test]
fn module_transient_work_py() {
    audit::check_scope("transient_work.py");
}
#[test]
fn module_visualization_py() {
    audit::check_scope("visualization.py");
}
