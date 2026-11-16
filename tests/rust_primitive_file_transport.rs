#[path = "support_primitive/file_transport.rs"]
mod file_transport;
#[path = "support_manager/mod.rs"]
mod support;
#[tokio::test]
async fn transport_d4a18731f140872d() {
    file_transport::run("test_a_file_crosses_the_chat_and_opens_on_the_other_side").await;
}
#[tokio::test]
async fn transport_50bfd112b55257c5() {
    file_transport::run("test_the_server_only_ever_sees_ciphertext").await;
}
#[tokio::test]
async fn transport_dcdb1b8b84d7c0c8() {
    file_transport::run("test_the_key_travels_inside_and_the_fingerprint_outside").await;
}
#[tokio::test]
async fn transport_90e7693deb1b717e() {
    file_transport::run("test_saving_a_file_whose_fingerprint_is_wrong_writes_nothing").await;
}
#[tokio::test]
async fn transport_450b66f7a7884d98() {
    file_transport::run("test_sending_a_file_refuses_on_a_chat_that_is_not_ready").await;
}
#[tokio::test]
async fn transport_ddcc75e7538ba567() {
    file_transport::run("test_sending_an_unreadable_file_refuses").await;
}
#[tokio::test]
async fn transport_de170c45b6a7f287() {
    file_transport::run("test_a_message_arrives_while_a_file_is_uploading").await;
}
#[tokio::test]
async fn transport_7145c7e6ae8e4571() {
    file_transport::run("test_a_text_sent_during_an_upload_lands_after_the_file").await;
}
#[tokio::test]
async fn transport_be05c7d73d6f81ae() {
    file_transport::run("test_a_multi_part_file_streams_both_ways_without_a_memory_copy").await;
}
