#![allow(clippy::unwrap_used)]
#[path = "support_security/error_contract.rs"]
mod contract;
#[path = "support_manager/mod.rs"]
mod manager_support;
#[path = "support_security/refusals.rs"]
mod refusals;
#[test]
fn test_no_error_carries_key_material_or_plaintext_parameterrejected() {
    let error = refusals::family("ParameterRejected");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_messagerejected() {
    let error = refusals::family("MessageRejected");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_chatnotready() {
    let error = refusals::family("ChatNotReady");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_chatclosed() {
    let error = refusals::family("ChatClosed");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_storagerequired() {
    let error = refusals::family("StorageRequired");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_layerunsupported() {
    let error = refusals::family("LayerUnsupported");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_resendunsatisfiable() {
    let error = refusals::family("ResendUnsatisfiable");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_storecorrupt() {
    let error = refusals::family("StoreCorrupt");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_unknownchat() {
    let error = refusals::family("UnknownChat");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_managerstopping() {
    let error = refusals::family("ManagerStopping");
    refusals::assert_private(&error);
}
#[test]
fn test_no_error_carries_key_material_or_plaintext_sendpending() {
    let error = refusals::family("SendPending");
    refusals::assert_private(&error);
}
#[test]
fn test_every_error_still_says_something_useful_parameterrejected() {
    let error = refusals::family("ParameterRejected");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_messagerejected() {
    let error = refusals::family("MessageRejected");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_chatnotready() {
    let error = refusals::family("ChatNotReady");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_chatclosed() {
    let error = refusals::family("ChatClosed");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_storagerequired() {
    let error = refusals::family("StorageRequired");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_layerunsupported() {
    let error = refusals::family("LayerUnsupported");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_resendunsatisfiable() {
    let error = refusals::family("ResendUnsatisfiable");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_storecorrupt() {
    let error = refusals::family("StoreCorrupt");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_unknownchat() {
    let error = refusals::family("UnknownChat");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_managerstopping() {
    let error = refusals::family("ManagerStopping");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_every_error_still_says_something_useful_sendpending() {
    let error = refusals::family("SendPending");
    refusals::assert_useful(&error, false);
}
#[test]
fn test_an_error_refuses_to_format_a_protocol_object() {
    contract::protocol_object_refusal();
}
#[test]
fn test_no_error_message_contains_a_long_hex_or_base64_run() {
    contract::all_errors_have_no_opaque_run();
}
#[test]
fn test_all_of_them_are_one_family() {
    contract::one_native_error_family();
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_dh_bad_residue() {
    let error = refusals::refusal("dh: bad residue");
    refusals::assert_site_shape("dh: bad residue", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_dh_composite_prime() {
    let error = refusals::refusal("dh: composite prime");
    refusals::assert_site_shape("dh: composite prime", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_dh_peer_value() {
    let error = refusals::refusal("dh: peer value");
    refusals::assert_site_shape("dh: peer value", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_crypto_wrong_fingerprint() {
    let error = refusals::refusal("crypto: wrong fingerprint");
    refusals::assert_site_shape("crypto: wrong fingerprint", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_crypto_tampered() {
    let error = refusals::refusal("crypto: tampered");
    refusals::assert_site_shape("crypto: tampered", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_crypto_truncated() {
    let error = refusals::refusal("crypto: truncated");
    refusals::assert_site_shape("crypto: truncated", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_framing_not_a_wrapper() {
    let error = refusals::refusal("framing: not a wrapper");
    refusals::assert_site_shape("framing: not a wrapper", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_framing_unparseable() {
    let error = refusals::refusal("framing: unparseable");
    refusals::assert_site_shape("framing: unparseable", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_framing_layer_too_low() {
    let error = refusals::refusal("framing: layer too low");
    refusals::assert_site_shape("framing: layer too low", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_parity() {
    let error = refusals::refusal("sequence: parity");
    refusals::assert_site_shape("sequence: parity", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_echo_backwards() {
    let error = refusals::refusal("sequence: echo backwards");
    refusals::assert_site_shape("sequence: echo backwards", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_echo_too_far() {
    let error = refusals::refusal("sequence: echo too far");
    refusals::assert_site_shape("sequence: echo too far", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_layer_backwards() {
    let error = refusals::refusal("sequence: layer backwards");
    refusals::assert_site_shape("sequence: layer backwards", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_resend_unsatisfiable() {
    let error = refusals::refusal("sequence: resend unsatisfiable");
    refusals::assert_site_shape("sequence: resend unsatisfiable", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_sequence_resend_too_wide() {
    let error = refusals::refusal("sequence: resend too wide");
    refusals::assert_site_shape("sequence: resend too wide", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_files_fingerprint() {
    let error = refusals::refusal("files: fingerprint");
    refusals::assert_site_shape("files: fingerprint", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_chat_not_ready() {
    let error = refusals::refusal("chat: not ready");
    refusals::assert_site_shape("chat: not ready", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_no_refusal_path_emits_key_material_or_plaintext_chat_closed() {
    let error = refusals::refusal("chat: closed");
    refusals::assert_site_shape("chat: closed", &error);
    refusals::assert_private(&error);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_dh_bad_residue() {
    let error = refusals::refusal("dh: bad residue");
    refusals::assert_site_shape("dh: bad residue", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_dh_composite_prime() {
    let error = refusals::refusal("dh: composite prime");
    refusals::assert_site_shape("dh: composite prime", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_dh_peer_value() {
    let error = refusals::refusal("dh: peer value");
    refusals::assert_site_shape("dh: peer value", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_crypto_wrong_fingerprint() {
    let error = refusals::refusal("crypto: wrong fingerprint");
    refusals::assert_site_shape("crypto: wrong fingerprint", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_crypto_tampered() {
    let error = refusals::refusal("crypto: tampered");
    refusals::assert_site_shape("crypto: tampered", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_crypto_truncated() {
    let error = refusals::refusal("crypto: truncated");
    refusals::assert_site_shape("crypto: truncated", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_framing_not_a_wrapper() {
    let error = refusals::refusal("framing: not a wrapper");
    refusals::assert_site_shape("framing: not a wrapper", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_framing_unparseable() {
    let error = refusals::refusal("framing: unparseable");
    refusals::assert_site_shape("framing: unparseable", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_framing_layer_too_low() {
    let error = refusals::refusal("framing: layer too low");
    refusals::assert_site_shape("framing: layer too low", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_parity() {
    let error = refusals::refusal("sequence: parity");
    refusals::assert_site_shape("sequence: parity", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_echo_backwards() {
    let error = refusals::refusal("sequence: echo backwards");
    refusals::assert_site_shape("sequence: echo backwards", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_echo_too_far() {
    let error = refusals::refusal("sequence: echo too far");
    refusals::assert_site_shape("sequence: echo too far", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_layer_backwards() {
    let error = refusals::refusal("sequence: layer backwards");
    refusals::assert_site_shape("sequence: layer backwards", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_resend_unsatisfiable() {
    let error = refusals::refusal("sequence: resend unsatisfiable");
    refusals::assert_site_shape("sequence: resend unsatisfiable", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_sequence_resend_too_wide() {
    let error = refusals::refusal("sequence: resend too wide");
    refusals::assert_site_shape("sequence: resend too wide", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_files_fingerprint() {
    let error = refusals::refusal("files: fingerprint");
    refusals::assert_site_shape("files: fingerprint", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_chat_not_ready() {
    let error = refusals::refusal("chat: not ready");
    refusals::assert_site_shape("chat: not ready", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_every_refusal_path_still_says_something_useful_chat_closed() {
    let error = refusals::refusal("chat: closed");
    refusals::assert_site_shape("chat: closed", &error);
    refusals::assert_useful(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_dh_bad_residue() {
    let error = refusals::refusal("dh: bad residue");
    refusals::assert_site_shape("dh: bad residue", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_dh_composite_prime() {
    let error = refusals::refusal("dh: composite prime");
    refusals::assert_site_shape("dh: composite prime", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_dh_peer_value() {
    let error = refusals::refusal("dh: peer value");
    refusals::assert_site_shape("dh: peer value", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_crypto_wrong_fingerprint() {
    let error = refusals::refusal("crypto: wrong fingerprint");
    refusals::assert_site_shape("crypto: wrong fingerprint", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_crypto_tampered() {
    let error = refusals::refusal("crypto: tampered");
    refusals::assert_site_shape("crypto: tampered", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_crypto_truncated() {
    let error = refusals::refusal("crypto: truncated");
    refusals::assert_site_shape("crypto: truncated", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_framing_not_a_wrapper() {
    let error = refusals::refusal("framing: not a wrapper");
    refusals::assert_site_shape("framing: not a wrapper", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_framing_unparseable() {
    let error = refusals::refusal("framing: unparseable");
    refusals::assert_site_shape("framing: unparseable", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_framing_layer_too_low() {
    let error = refusals::refusal("framing: layer too low");
    refusals::assert_site_shape("framing: layer too low", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_parity() {
    let error = refusals::refusal("sequence: parity");
    refusals::assert_site_shape("sequence: parity", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_echo_backwards() {
    let error = refusals::refusal("sequence: echo backwards");
    refusals::assert_site_shape("sequence: echo backwards", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_echo_too_far() {
    let error = refusals::refusal("sequence: echo too far");
    refusals::assert_site_shape("sequence: echo too far", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_layer_backwards() {
    let error = refusals::refusal("sequence: layer backwards");
    refusals::assert_site_shape("sequence: layer backwards", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_resend_unsatisfiable() {
    let error = refusals::refusal("sequence: resend unsatisfiable");
    refusals::assert_site_shape("sequence: resend unsatisfiable", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_sequence_resend_too_wide() {
    let error = refusals::refusal("sequence: resend too wide");
    refusals::assert_site_shape("sequence: resend too wide", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_files_fingerprint() {
    let error = refusals::refusal("files: fingerprint");
    refusals::assert_site_shape("files: fingerprint", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_chat_not_ready() {
    let error = refusals::refusal("chat: not ready");
    refusals::assert_site_shape("chat: not ready", &error);
    refusals::assert_no_opaque(&error, true);
}
#[test]
fn test_no_refusal_path_emits_a_long_opaque_run_chat_closed() {
    let error = refusals::refusal("chat: closed");
    refusals::assert_site_shape("chat: closed", &error);
    refusals::assert_no_opaque(&error, true);
}
