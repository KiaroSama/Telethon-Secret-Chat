#[path = "support_rekey/core.rs"]
mod core;
#[path = "support_primitive/dh.rs"]
mod dh_material;
#[path = "support_rekey/material.rs"]
mod material;
#[test]
fn rekey_875a784530c6327e() {
    core::run("test_a_fresh_chat_does_not_rekey");
}
#[test]
fn rekey_87a665ac0b96bb54() {
    core::run("test_a_hundred_messages_triggers_it");
}
#[test]
fn rekey_7eac63e49211454f() {
    core::run("test_a_week_triggers_it");
}
#[test]
fn rekey_ce96f492ea547dae() {
    core::run("test_an_old_but_unused_key_does_not_trigger_it");
}
#[test]
fn rekey_432b7e24af593658() {
    core::run("test_an_exchange_already_running_does_not_trigger_another");
}
#[test]
fn rekey_825126dbeb977fb0() {
    core::run("test_a_retained_previous_key_does_not_trigger_another");
}
#[test]
fn rekey_fda492ddbecb1b15() {
    core::run("test_the_exchange_id_is_a_full_width_random_int64");
}
#[test]
fn rekey_0737056bea308f6f() {
    core::run("test_the_exchange_id_does_not_come_from_the_random_module");
}
#[test]
fn rekey_e9e0a540f9ffdde0() {
    core::run("test_the_documented_tie_break[100-50-abandon_theirs]");
}
#[test]
fn rekey_2f8b25beefcda6d3() {
    core::run("test_the_documented_tie_break[50-100-join_theirs]");
}
#[test]
fn rekey_6c426151099504f3() {
    core::run("test_the_documented_tie_break[50-50-abort_both]");
}
#[test]
fn rekey_e3f66683eca55df2() {
    core::run("test_an_exchange_may_be_aborted_before_a_commitment");
}
#[test]
fn rekey_dd8c94eb5c11538d() {
    core::run("test_an_exchange_may_not_be_aborted_after_one[accepted]");
}
#[test]
fn rekey_f5b4b21ea8edab29() {
    core::run("test_an_exchange_may_not_be_aborted_after_one[committed]");
}
#[test]
fn rekey_893429a728fdc709() {
    core::run("test_a_frame_is_matched_to_whichever_held_key_it_names");
}
#[test]
fn rekey_2a5d3a768d6d9764() {
    core::run("test_the_pending_key_can_decrypt_too");
}
#[test]
fn rekey_6c584f121a0cf329() {
    core::run("test_the_counters_are_not_reset_by_a_rekey");
}
#[test]
fn rekey_07d186bc0b2c1bfb() {
    core::run("test_adopting_a_new_key_retains_the_old_one");
}
#[test]
fn rekey_29218976ba3139db() {
    core::run("test_the_previous_key_is_dropped_once_the_new_one_has_carried_a_message");
}
#[tokio::test]
async fn rekey_e80dd4ebb926e0dd() {
    core::run("test_the_old_key_is_kept_while_a_gap_is_open");
}
#[tokio::test]
async fn rekey_f16ceca753e35e25() {
    core::run("test_actions_for_a_foreign_exchange_are_ignored_and_not_applied[action0]");
}
#[tokio::test]
async fn rekey_3b16c6099b75be86() {
    core::run("test_actions_for_a_foreign_exchange_are_ignored_and_not_applied[action1]");
}
#[tokio::test]
async fn rekey_fc5b83180508b92a() {
    core::run("test_actions_for_a_foreign_exchange_are_ignored_and_not_applied[action2]");
}
#[tokio::test]
async fn rekey_d57c513740d71bdf() {
    core::run("test_a_request_key_while_the_previous_key_is_held_is_answered_with_abort");
}
