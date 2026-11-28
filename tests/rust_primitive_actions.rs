#[path = "support_primitive/actions.rs"]
mod action_cases;
#[test]
fn action_f854ac02a2b616e9() {
    action_cases::run("test_there_are_thirteen_and_the_test_covers_them_all");
}
#[test]
fn action_f812316568b1889e() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionSetMessageTTL]",
    );
}
#[test]
fn action_2e180c18dd44035e() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionReadMessages]",
    );
}
#[test]
fn action_7a9a7e670a97a90f() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionDeleteMessages]",
    );
}
#[test]
fn action_d49c68d7e0a5acf0() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionScreenshotMessages]",
    );
}
#[test]
fn action_5162d96d5b478142() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionFlushHistory]",
    );
}
#[test]
fn action_28b95ab16a729dd2() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionResend]");
}
#[test]
fn action_f577750e5489cfe0() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionNotifyLayer]",
    );
}
#[test]
fn action_65ae428020e55cb2() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionTyping]");
}
#[test]
fn action_f2c371dce821d3ee() {
    action_cases::run(
        "test_every_action_round_trips_on_the_wire[decryptedMessageActionRequestKey]",
    );
}
#[test]
fn action_9a06e8dad8ccf41c() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionAcceptKey]");
}
#[test]
fn action_fc7aacc979cfd5cd() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionCommitKey]");
}
#[test]
fn action_d11040a313572547() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionAbortKey]");
}
#[test]
fn action_ca7549bf6dffc60d() {
    action_cases::run("test_every_action_round_trips_on_the_wire[decryptedMessageActionNoop]");
}
#[test]
fn action_5ec82cdab3089963() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionSetMessageTTL]");
}
#[test]
fn action_cfd5c4395f79170c() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionReadMessages]");
}
#[test]
fn action_cd64d6504d8e867e() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionDeleteMessages]");
}
#[test]
fn action_a71a56866132654f() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionScreenshotMessages]");
}
#[test]
fn action_0f08d065371f3798() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionFlushHistory]");
}
#[test]
fn action_8b4fa7965448af28() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionResend]");
}
#[test]
fn action_0c56e3f2abe715dc() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionNotifyLayer]");
}
#[test]
fn action_4c929377f558e5bb() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionTyping]");
}
#[test]
fn action_88e061377016de4d() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionRequestKey]");
}
#[test]
fn action_c5894a26cbc8e661() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionAcceptKey]");
}
#[test]
fn action_d735faae417ae3b4() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionCommitKey]");
}
#[test]
fn action_a1ba99e24e9aa72e() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionAbortKey]");
}
#[test]
fn action_0e3ce0e98b9f2ca3() {
    action_cases::run("test_every_action_is_classified[decryptedMessageActionNoop]");
}
#[test]
fn action_21b4b1b2c5f893b1() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionSetMessageTTL-conversation]",
    );
}
#[test]
fn action_f031baa17963d370() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionReadMessages-conversation]",
    );
}
#[test]
fn action_e2f5513678698d45() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionDeleteMessages-conversation]",
    );
}
#[test]
fn action_67365dd285a44e0e() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionFlushHistory-conversation]",
    );
}
#[test]
fn action_67ae59c950c4c94d() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionScreenshotMessages-conversation]",
    );
}
#[test]
fn action_674adf808c844b40() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionTyping-conversation]",
    );
}
#[test]
fn action_370de29bbef8c32e() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionNotifyLayer-protocol]",
    );
}
#[test]
fn action_4ffe28f15a607cd6() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionResend-protocol]",
    );
}
#[test]
fn action_dd439ec9ccc1a386() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionNoop-protocol]",
    );
}
#[test]
fn action_63e3ec7bc923e358() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionRequestKey-rekey]",
    );
}
#[test]
fn action_eb2b45e54b92da7c() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionAcceptKey-rekey]",
    );
}
#[test]
fn action_9b8123d20f109a15() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionCommitKey-rekey]",
    );
}
#[test]
fn action_b0ecc7381370b268() {
    action_cases::run(
        "test_the_groups_are_as_the_data_model_states[decryptedMessageActionAbortKey-rekey]",
    );
}
