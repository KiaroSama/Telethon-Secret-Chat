#[path = "support_primitive/framing.rs"]
mod framing;
#[test]
fn framing_645b5d2b4051978f() {
    framing::run("test_a_message_is_wrapped_in_decrypted_message_layer");
}
#[test]
fn framing_9c9c34ba4a4420c6() {
    framing::run("test_a_service_message_is_wrapped_too");
}
#[test]
fn framing_c24693458f80bedf() {
    framing::run("test_a_wrapped_message_survives_serialization");
}
#[test]
fn framing_7e26a8c2ccc7e435() {
    framing::run("test_random_bytes_is_thirty_one_bytes");
}
#[test]
fn framing_8341caecb3e18c0d() {
    framing::run("test_random_bytes_differs_every_time");
}
#[test]
fn framing_0aeb4f20dfdbb91f() {
    framing::run("test_a_wrapper_below_layer_46_is_refused");
}
#[test]
fn framing_5d423742d30863ee() {
    framing::run("test_a_body_that_is_not_a_wrapper_is_refused");
}
#[test]
fn framing_68af455201bf79bd() {
    framing::run("test_an_unparseable_body_is_refused_not_guessed");
}
#[test]
fn framing_4bd97a968e2b1f42() {
    framing::run("test_the_remote_layer_starts_at_46");
}
#[test]
fn framing_3b8ac7b2a9175066() {
    framing::run("test_the_remote_layer_only_ever_rises");
}
#[test]
fn framing_606b6a0453b991a4() {
    framing::run("test_the_outgoing_layer_is_clamped_between_73_and_144[46-73]");
}
#[test]
fn framing_c59ec00888beb1d5() {
    framing::run("test_the_outgoing_layer_is_clamped_between_73_and_144[73-73]");
}
#[test]
fn framing_cf46f8b716913dc8() {
    framing::run("test_the_outgoing_layer_is_clamped_between_73_and_144[101-101]");
}
#[test]
fn framing_1de2a2df68daf066() {
    framing::run("test_the_outgoing_layer_is_clamped_between_73_and_144[144-144]");
}
#[test]
fn framing_639ac5c00523664b() {
    framing::run("test_the_outgoing_layer_is_clamped_between_73_and_144[223-144]");
}
#[test]
fn framing_40458c57407f09ab() {
    framing::run("test_a_peer_that_cannot_reach_73_is_refused");
}
#[test]
fn framing_6b11dc582b8ad47f() {
    framing::run("test_the_seq_no_transform[True-0-0-0-1]");
}
#[test]
fn framing_4d8136f18dd683b9() {
    framing::run("test_the_seq_no_transform[True-3-5-6-11]");
}
#[test]
fn framing_df903c2cbd1910e7() {
    framing::run("test_the_seq_no_transform[False-0-0-1-0]");
}
#[test]
fn framing_9c96edd1f8ce6a5e() {
    framing::run("test_the_seq_no_transform[False-3-5-7-10]");
}
#[test]
fn framing_cba45166b3432f39() {
    framing::run("test_the_transform_is_reversible");
}
