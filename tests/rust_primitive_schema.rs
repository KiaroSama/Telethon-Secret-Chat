#[path = "support_primitive/compile_boundary.rs"]
mod compile_boundary;
#[path = "support_primitive/schema_oracle.rs"]
mod oracle;
#[path = "support_primitive/schema.rs"]
mod schema;
#[path = "support_primitive/wire_properties.rs"]
mod wire_properties;
#[test]
fn schema_4b44317cbb8b0df9() {
    schema::run("test_every_published_constructor_is_registered");
    oracle::check_all();
}
#[test]
fn schema_0637d99781ffc163() {
    schema::run("test_no_constructor_was_invented");
}
#[test]
fn schema_5399c81674c9505d() {
    schema::run("test_the_ids_the_protocol_reference_quotes_are_the_ones_generated");
}
#[test]
fn schema_059e2c0bf8ea7bcf() {
    schema::run("test_all_thirteen_actions_exist");
}
#[test]
fn schema_c10118a74044c3bf() {
    schema::run("test_a_plain_field_object_round_trips");
}
#[test]
fn schema_4386467356801f65() {
    schema::run("test_a_vector_of_longs_round_trips");
}
#[test]
fn schema_9c969c15b08de8f1() {
    schema::run("test_an_empty_constructor_round_trips");
}
#[test]
fn schema_9a5ae7fa67052f5d() {
    schema::run("test_bytes_fields_survive_their_padding");
}
#[test]
fn schema_bc49f47c12c9e1ed() {
    schema::run("test_a_string_field_round_trips_as_utf8");
}
#[test]
fn schema_4fe946f3f5585314() {
    schema::run("test_an_absent_flag_field_stays_absent");
}
#[test]
fn schema_2f5229cf5f80772d() {
    schema::run("test_a_present_flag_field_comes_back");
}
#[test]
fn schema_5c91397c0f7a4575() {
    schema::run("test_a_true_flag_is_carried_by_its_bit_alone");
}
#[test]
fn schema_f569375a139b0b04() {
    schema::run("test_a_nested_object_round_trips");
}
#[test]
fn schema_fbc4aa9bfcc28376() {
    schema::run("test_a_vector_of_objects_round_trips");
}
#[test]
fn schema_db827661579989dc() {
    schema::run("test_the_layer_8_service_shape_is_accepted");
}
#[test]
fn schema_337a158e457814ff() {
    schema::run("test_an_unknown_constructor_is_reported_not_guessed");
}
#[test]
fn schema_81055643db9d14db() {
    for format in ["{value:?}", "{value}"] {
        compile_boundary::refuses_program(
            &format!(
                "pub fn check(value: kiaro_secret_chat::enums::DecryptedMessage) {{ let _ = format!({format:?}); }}"
            ),
            "E0277",
        );
    }
}
#[test]
fn schema_1259208695dbe0ff() {
    schema::run("test_a_nested_layer_is_refused_at_the_declared_message_type_boundary");
}
#[test]
fn schema_cd810ef567e13fa9() {
    schema::run("test_untyped_root_parsing_still_enforces_the_depth_bound");
}
