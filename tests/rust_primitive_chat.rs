#![allow(clippy::unwrap_used)]
#[path = "support_primitive/chat.rs"]
mod chat;
#[test]
fn chat_34d95815f1e105fd() {
    chat::run("test_an_outbound_chat_starts_requested");
}
#[test]
fn chat_016d355ecabf216f() {
    chat::run("test_an_inbound_chat_starts_pending");
}
#[test]
fn chat_0f50271467a3259e() {
    chat::run("test_the_legal_transitions_are_allowed[requested-ready]");
}
#[test]
fn chat_03db5d79c55de305() {
    chat::run("test_the_legal_transitions_are_allowed[requested-closed]");
}
#[test]
fn chat_27defe0c3d474220() {
    chat::run("test_the_legal_transitions_are_allowed[pending-ready]");
}
#[test]
fn chat_5be7a603378eac26() {
    chat::run("test_the_legal_transitions_are_allowed[pending-closed]");
}
#[test]
fn chat_fc60af3c5d2e25d7() {
    chat::run("test_the_legal_transitions_are_allowed[ready-rekeying]");
}
#[test]
fn chat_db83d247777d8827() {
    chat::run("test_the_legal_transitions_are_allowed[ready-closed]");
}
#[test]
fn chat_4f3618d0eb88627e() {
    chat::run("test_the_legal_transitions_are_allowed[rekeying-ready]");
}
#[test]
fn chat_48f6b34b21815c0c() {
    chat::run("test_the_legal_transitions_are_allowed[rekeying-closed]");
}
#[test]
fn chat_22f5d7b34237aa59() {
    chat::run("test_the_illegal_transitions_are_refused[requested-pending]");
}
#[test]
fn chat_054258635ab22798() {
    chat::run("test_the_illegal_transitions_are_refused[requested-rekeying]");
}
#[test]
fn chat_13b606622b87e2a1() {
    chat::run("test_the_illegal_transitions_are_refused[pending-requested]");
}
#[test]
fn chat_2944a5729b6cd4c0() {
    chat::run("test_the_illegal_transitions_are_refused[ready-requested]");
}
#[test]
fn chat_7c17e146d933fc9c() {
    chat::run("test_the_illegal_transitions_are_refused[ready-pending]");
}
#[test]
fn chat_1496eb4889e681af() {
    chat::run("test_the_illegal_transitions_are_refused[rekeying-pending]");
}
#[test]
fn chat_73b90b89875d6685() {
    chat::run("test_closed_is_terminal[requested]");
}
#[test]
fn chat_732839152071e125() {
    chat::run("test_closed_is_terminal[pending]");
}
#[test]
fn chat_c94804b5e169429f() {
    chat::run("test_closed_is_terminal[ready]");
}
#[test]
fn chat_e71db7490dbf3e64() {
    chat::run("test_closed_is_terminal[rekeying]");
}
#[test]
fn chat_bc5e5344bbdfc071() {
    chat::run("test_closed_is_terminal[closed]");
}
#[test]
fn chat_9b73e61a5f6629ad() {
    chat::run("test_send_works_in_ready_and_in_rekeying[ready]");
}
#[test]
fn chat_edf48fb69549b611() {
    chat::run("test_send_works_in_ready_and_in_rekeying[rekeying]");
}
#[test]
fn chat_cd2a76e846dd0585() {
    chat::run("test_send_refuses_before_the_key_exists[requested]");
}
#[test]
fn chat_c247ee5a6e222109() {
    chat::run("test_send_refuses_before_the_key_exists[pending]");
}
#[test]
fn chat_46c21ac4c83f2636() {
    chat::run("test_send_refuses_on_a_closed_chat_with_a_stated_reason");
}
#[test]
fn chat_3af82a49747a725c() {
    chat::run("test_adopting_a_key_sets_its_fingerprint_in_the_same_call");
}
#[test]
fn chat_73979d6afb8194cd() {
    chat::run("test_a_key_of_the_wrong_length_is_refused");
}
#[test]
fn chat_36757432cf5ab541() {
    chat::run("test_the_key_and_fingerprint_cannot_drift_apart");
}
#[test]
fn chat_8bc15597e3365f7b() {
    chat::run("test_the_crypto_x_follows_the_side");
}
#[test]
fn chat_b788fd0f28957b15() {
    chat::run("test_a_chat_round_trips_through_its_record");
}
#[test]
fn chat_6a3655bbdb75ca13() {
    chat::run("test_the_record_carries_every_field_the_data_model_lists");
}
#[test]
fn chat_7d9061563a8ec18c() {
    chat::run("test_a_chat_never_prints_its_key");
}
