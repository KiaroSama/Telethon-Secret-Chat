#[path = "support_rekey/manager.rs"]
mod manager;
#[path = "support_rekey/material.rs"]
mod material;
#[path = "support_manager/mod.rs"]
mod support;
#[tokio::test]
async fn rekey_a796a4db487dc865() {
    manager::run("test_an_exchange_completes_and_both_ends_agree_on_the_new_key").await;
}
#[tokio::test]
async fn rekey_92ef40bceeea761d() {
    manager::run("test_the_conversation_continues_after_the_exchange").await;
}
#[tokio::test]
async fn rekey_3d054a89a58cceec() {
    manager::run("test_a_message_sent_mid_exchange_is_delivered").await;
}
#[tokio::test]
async fn rekey_f67f184452e98eca() {
    manager::run("test_a_peers_abort_clears_the_local_exchange").await;
}
#[tokio::test]
async fn rekey_b510cad01e4a48a3() {
    manager::run("test_a_bad_fingerprint_in_an_accept_does_not_switch_the_key").await;
}
#[tokio::test]
async fn rekey_1b8b8dde90bc5dfe() {
    manager::run("test_a_restart_during_an_exchange_leaves_the_chat_usable").await;
}
#[tokio::test]
async fn rekey_513d9792870323bc() {
    manager::run("test_the_previous_key_is_discarded_once_the_new_one_is_in_use").await;
}
#[tokio::test]
async fn rekey_823747a473ac132c() {
    manager::run("test_a_request_key_with_an_unsafe_value_closes_the_chat").await;
}
#[tokio::test]
async fn rekey_9bcf89063220fc9d() {
    manager::run("test_flushing_history_does_not_destroy_a_pending_rekey_step").await;
}
#[tokio::test]
async fn rekey_536de76dc680d98d() {
    manager::run("test_a_peer_flush_leaves_pending_service_actions_alone").await;
}
#[tokio::test]
async fn rekey_38327f4892423dc7() {
    manager::run("test_both_sides_requesting_at_once_end_on_one_key").await;
}
#[tokio::test]
async fn rekey_6407182b722cf536() {
    manager::run("test_a_commit_with_the_wrong_fingerprint_closes_the_chat").await;
}
#[tokio::test]
async fn rekey_f4b0334554e52d4b() {
    manager::run("test_crossing_requests_settle_on_one_key_with_concurrent_dispatch").await;
}
#[tokio::test]
async fn rekey_775c6096b4326328() {
    manager::run("test_a_rekey_completes_with_concurrent_dispatch").await;
}
