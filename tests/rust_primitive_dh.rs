#[path = "support_primitive/dh.rs"]
mod dh;
#[test]
fn dh_47a60ebc3805c4d2() {
    dh::parameters("test_a_real_safe_prime_with_g_2_is_accepted");
}
#[test]
fn dh_97b1a173ccc5ab52() {
    dh::parameters("test_a_composite_prime_is_refused");
}
#[test]
fn dh_ddd0ad735e8e3738() {
    dh::parameters("test_the_residue_condition_is_what_refuses_a_prime_of_the_wrong_class");
}
#[test]
fn dh_acc70b9ac86dc466() {
    dh::parameters("test_a_prime_of_the_wrong_bit_length_is_refused");
}
#[test]
fn dh_5a481a2cc25e82dc() {
    dh::parameters("test_a_prime_one_bit_too_large_is_refused");
}
#[test]
fn dh_c56e5fbe3f6fe145() {
    dh::parameters("test_a_prime_whose_half_is_composite_is_refused");
}
#[test]
fn dh_3c89bc97481dbcf3() {
    dh::parameters("test_the_primality_test_agrees_with_a_known_sieve");
}
#[test]
fn dh_fbf639f129aaf4de() {
    dh::parameters("test_a_generator_at_or_below_one_is_refused[0]");
}
#[test]
fn dh_1b801760dd420020() {
    dh::parameters("test_a_generator_at_or_below_one_is_refused[1]");
}
#[test]
fn dh_639830af079a0bd4() {
    dh::parameters("test_a_generator_at_or_below_one_is_refused[-5]");
}
#[test]
fn dh_ace29186fec18210() {
    dh::parameters("test_a_generator_at_or_above_p_minus_one_is_refused");
}
#[test]
fn dh_207c9213638cba89() {
    dh::parameters("test_a_generator_outside_the_documented_set_is_refused");
}
#[test]
fn dh_7ac1f23b7ad5454b() {
    dh::parameters("test_g_2_against_a_prime_failing_its_residue_condition");
}
#[test]
fn dh_371cdaaf0e07cf33() {
    dh::parameters("test_every_documented_residue_condition[2-7-1]");
}
#[test]
fn dh_9eb9836a8ecd5aea() {
    dh::parameters("test_every_documented_residue_condition[3-2-1]");
}
#[test]
fn dh_5a9938f863eda1ac() {
    dh::parameters("test_every_documented_residue_condition[5-1-2]");
}
#[test]
fn dh_7e56c5860517e9c5() {
    dh::parameters("test_every_documented_residue_condition[6-19-5]");
}
#[test]
fn dh_c20737e5ed10d0d7() {
    dh::parameters("test_every_documented_residue_condition[7-3-1]");
}
#[test]
fn dh_3c8503133783d2f2() {
    dh::parameters("test_g_4_has_no_condition");
}
#[test]
fn dh_64649cf5289f580b() {
    dh::parameters("test_a_structurally_wrong_prime_is_refused_before_any_exponentiation");
}
#[test]
fn dh_cbb2f7d624519121() {
    dh::parameters("test_the_refusal_names_the_chat_without_naming_the_value");
}
