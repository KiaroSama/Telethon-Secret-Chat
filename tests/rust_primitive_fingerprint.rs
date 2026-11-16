#[path = "support_primitive/dh.rs"]
mod dh;
#[test]
fn fingerprint_d46dc8c8ae09bf79() {
    dh::fingerprint("test_the_fingerprint_is_sha1_of_the_key_last_eight_bytes_little_endian");
}
#[test]
fn fingerprint_4b061d4ab1140ef6() {
    dh::fingerprint("test_the_fingerprint_fits_a_signed_int64");
}
#[test]
fn fingerprint_f2a54fe9621f2f98() {
    dh::fingerprint("test_a_different_key_gives_a_different_fingerprint");
}
#[test]
fn fingerprint_74aa512d70309625() {
    dh::fingerprint("test_both_sides_derive_the_same_key_and_fingerprint");
}
#[test]
fn fingerprint_0091a52153d9cd58() {
    dh::fingerprint("test_the_shared_key_is_always_256_bytes");
}
#[test]
fn fingerprint_6e4902b9f8979dd5() {
    dh::fingerprint("test_the_padding_is_on_the_LEFT");
}
#[test]
fn fingerprint_2be6524948cd4bb7() {
    dh::fingerprint("test_the_secret_is_2048_bits_from_a_secure_source");
}
#[test]
fn fingerprint_bebd253a71c85a28() {
    dh::fingerprint("test_deriving_a_key_validates_the_peer_value_first");
}
#[test]
fn fingerprint_a1eec3b8641ddac7() {
    dh::fingerprint("test_the_fingerprint_comparison_refuses_a_mismatch");
}
#[test]
fn fingerprint_01e22514b47e29ce() {
    dh::fingerprint("test_a_fingerprint_mismatch_does_not_report_either_value");
}
