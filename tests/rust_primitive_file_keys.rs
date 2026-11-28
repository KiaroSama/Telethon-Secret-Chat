#[path = "support_primitive/file_keys.rs"]
mod file_keys;
#[test]
fn file_9149ed5841432b06() {
    file_keys::run("test_a_file_key_is_thirty_two_bytes_and_so_is_its_iv");
}
#[test]
fn file_9d3d3419a7cdc86d() {
    file_keys::run("test_every_file_gets_its_own_key");
}
#[test]
fn file_1d8ad4df1ef2bd85() {
    file_keys::run("test_the_file_key_is_unrelated_to_the_chat_key");
}
#[test]
fn file_14f06f4862e04898() {
    file_keys::run("test_the_file_key_comes_from_a_secure_source");
}
#[test]
fn file_218a66d55b5c78ec() {
    file_keys::run("test_the_fingerprint_is_the_documented_md5_fold");
}
#[test]
fn file_71c8beabf5c6d190() {
    file_keys::run("test_it_is_md5_and_not_the_sha1_of_section_1_5");
}
#[test]
fn file_f469c93018e467c2() {
    file_keys::run("test_it_fits_a_signed_int32");
}
#[test]
fn file_8690c46667beaa18() {
    file_keys::run("test_a_different_key_or_iv_changes_it");
}
#[test]
fn file_038a89cf5edba221() {
    file_keys::run("test_a_file_round_trips_through_its_own_key");
}
#[test]
fn file_b84c9add99c9513a() {
    file_keys::run("test_the_ciphertext_is_padded_to_a_cipher_block");
}
#[test]
fn file_625005fa8bc8ccfd() {
    file_keys::run("test_the_padding_is_removed_on_the_way_back");
}
#[test]
fn file_91b13ddb8a017b66() {
    file_keys::run("test_a_file_encrypted_under_one_key_does_not_open_under_another");
}
#[test]
fn file_4dcbecdf67b7e050() {
    file_keys::run("test_a_mismatched_fingerprint_is_refused");
}
#[test]
fn file_4405d76ecf656d49() {
    file_keys::run("test_a_matching_fingerprint_is_accepted");
}
#[test]
fn file_a60040e7d63e9046() {
    file_keys::run("test_nothing_is_written_when_the_fingerprint_does_not_match");
}
#[test]
fn file_68fa1a3dbaa0c149() {
    file_keys::run("test_a_good_file_is_written");
}
#[test]
fn file_cc9eb0be83628f6d() {
    file_keys::run("test_the_refusal_names_no_key_and_no_content");
}
#[test]
fn file_eb2d965cca1d1d6a() {
    file_keys::run("test_a_leftover_media_temp_file_is_removed_before_saving");
}
