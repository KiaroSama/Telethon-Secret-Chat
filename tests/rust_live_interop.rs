//! Exactly ten opt-in real-account scenarios. CI must collect and explicitly
//! ignore these; it must never substitute a fake transport or open credentials.
mod support_live;
mod support_live_harness;
use support_live_harness::{
    TestResult,
    cases::{self, Official, Paired},
};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent, dedicated existing test authorization and an official client"]
async fn both_ends_agree_on_key_fingerprint() -> TestResult {
    cases::official(Official::Fingerprint).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent, dedicated existing test authorization and an official client"]
async fn message_crosses_both_directions() -> TestResult {
    cases::official(Official::RoundTrip).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and dedicated existing test authorization"]
async fn closing_here_refuses_later_send() -> TestResult {
    cases::official(Official::Closed).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent, dedicated test authorization and operator media confirmation"]
async fn media_opened_by_official_client() -> TestResult {
    cases::official(Official::MediaOut).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and a real file from an official client"]
async fn incoming_file_decrypts_and_is_written() -> TestResult {
    cases::official(Official::MediaIn).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and an official client accepting rekey"]
async fn official_client_accepts_rekey() -> TestResult {
    cases::official(Official::Rekey).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and two dedicated existing test authorizations"]
async fn pair_text_rekey_files_and_restart() -> TestResult {
    cases::paired(Paired::Lifecycle).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and two dedicated existing test authorizations"]
async fn peer_close_arrives_on_update_sync() -> TestResult {
    cases::paired(Paired::CloseSync).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent and two dedicated existing test authorizations"]
async fn sending_after_peer_close_ends_local_chat() -> TestResult {
    cases::paired(Paired::CloseSend).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit consent to delete the throwaway chat on both dedicated test accounts"]
async fn delete_both_sides_removes_peer_chat() -> TestResult {
    cases::paired(Paired::DeleteBoth).await
}
