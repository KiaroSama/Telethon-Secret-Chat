#[path = "support_sdk/boundary.rs"]
mod boundary;
#[path = "support_sdk/machine.rs"]
mod machine;
mod support_sdk;
#[tokio::test]
async fn real_original_client_and_live_receiver_commit_before_deleting_server_ack() {
    support_sdk::run("adapter");
    boundary::original_handle_clone_refuses_before_request_is_queued().await;
    machine::live_codec_retains_complete_envelope_and_peers();
    support_sdk::run("bot");
    support_sdk::run("live");
}
#[tokio::test]
async fn real_original_client_and_difference_receiver_commit_before_deleting_server_ack() {
    boundary::receiver_restores_checkpoint_before_initial_state_rpc().await;
    machine::checkpoint_restores_counter_and_difference_ownership();
    machine::checkpoint_rejects_trailing_bytes_and_truncation();
    support_sdk::run("difference");
}
#[test]
fn real_explicit_ack_deletes_only_committed_frontier() {
    support_sdk::run("explicit");
}
#[test]
fn failed_receipt_commit_preserves_deleting_server_event_and_old_checkpoint() {
    support_sdk::run("capacity");
}
