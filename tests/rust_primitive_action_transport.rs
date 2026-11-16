#[path = "support_primitive/actions.rs"]
mod action_cases;
#[path = "support_primitive/action_transport.rs"]
mod action_transport;
#[path = "support_manager/mod.rs"]
mod support;
#[tokio::test]
async fn action_66e742567997b0ae() {
    action_transport::run("test_the_peers_ttl_is_stored").await;
}
#[tokio::test]
async fn action_370fc9ddede18d48() {
    action_transport::run("test_a_ttl_of_zero_disables_it").await;
}
#[tokio::test]
async fn action_24cdc7e9a94d53b3() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionSetMessageTTL]",
    )
    .await;
}
#[tokio::test]
async fn action_bf709ce2172a641a() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionReadMessages]",
    )
    .await;
}
#[tokio::test]
async fn action_763522e1ea7dd83f() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionDeleteMessages]",
    )
    .await;
}
#[tokio::test]
async fn action_2b3055f192fb889d() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionScreenshotMessages]",
    )
    .await;
}
#[tokio::test]
async fn action_097695d5f25a3cd3() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionFlushHistory]",
    )
    .await;
}
#[tokio::test]
async fn action_a7b38205063130cb() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionResend]",
    )
    .await;
}
#[tokio::test]
async fn action_0f956d347b3c0371() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionNotifyLayer]",
    )
    .await;
}
#[tokio::test]
async fn action_70d9099baf232aba() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionTyping]",
    )
    .await;
}
#[tokio::test]
async fn action_afab60c7d227ee95() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionRequestKey]",
    )
    .await;
}
#[tokio::test]
async fn action_d84b4acff6a25cce() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionAcceptKey]",
    )
    .await;
}
#[tokio::test]
async fn action_ec6b53c2da8694e5() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionCommitKey]",
    )
    .await;
}
#[tokio::test]
async fn action_ea46270cfc50129d() {
    action_transport::run(
        "test_every_action_reaches_the_application[decryptedMessageActionAbortKey]",
    )
    .await;
}
#[tokio::test]
async fn action_bc991458a05cd8f0() {
    action_transport::run("test_every_action_reaches_the_application[decryptedMessageActionNoop]")
        .await;
}
#[tokio::test]
async fn action_5201fd7dab6b84ea() {
    action_transport::run("test_an_unknown_action_is_reported_rather_than_dropped").await;
}
#[tokio::test]
async fn action_5b67ffb35cdc9402() {
    action_transport::run("test_the_six_the_consumer_needs_all_have_a_send_path").await;
}
#[tokio::test]
async fn action_0344d6d7d9ec521f() {
    action_transport::run("test_setting_a_ttl_stores_it_on_this_side_too").await;
}
#[tokio::test]
async fn action_a16fc40be9f48164() {
    action_transport::run("test_a_message_sent_under_a_ttl_carries_it").await;
}
#[tokio::test]
async fn action_76a417777d3ac6f8() {
    action_transport::run("test_a_service_action_advances_the_counter").await;
}
#[tokio::test]
async fn action_acc9f8c0bd6d3598() {
    action_transport::run("test_deleting_an_unacknowledged_message_leaves_no_hole").await;
}
