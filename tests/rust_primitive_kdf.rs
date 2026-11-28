#[path = "support_primitive/frame_properties.rs"]
mod frame_properties;
#[path = "support_primitive/grammers_kdf.rs"]
mod grammers_kdf;
#[path = "support_primitive/kdf.rs"]
mod kdf;
#[test]
fn kdf_eb93148fa5b6bc5c() {
    kdf::fixed(0);
}
#[test]
fn kdf_f8676f7c825e7446() {
    kdf::fixed(8);
}
#[test]
fn kdf_6870602758210b5e() {
    kdf::random(0);
}
#[test]
fn kdf_723cdaff2a71aba7() {
    kdf::random(8);
}
#[test]
fn kdf_61f0b2add8546ee4() {
    kdf::other("test_the_two_directions_do_not_share_a_key");
}
#[test]
fn kdf_0f7bf56b646e4f5b() {
    kdf::other("test_an_x_outside_the_protocol_is_refused");
}
#[test]
fn kdf_ef96a94f408b1bbb() {
    kdf::other("test_the_key_and_iv_are_the_sizes_aes_256_ige_needs");
}
#[test]
fn kdf_c556ca971c2b6b68() {
    kdf::other("test_msg_key_is_the_middle_128_bits_of_the_documented_hash");
}
#[test]
fn kdf_40c0fd6ec2306596() {
    kdf::other("test_msg_key_covers_the_padding_too");
}
#[test]
fn kdf_c0baccfa19161095() {
    kdf::other("test_the_fingerprint_is_the_last_64_bits_of_sha1_little_endian");
}
