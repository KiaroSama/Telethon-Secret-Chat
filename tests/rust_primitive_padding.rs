#[path = "support_primitive/padding.rs"]
mod padding;
#[test]
fn padding_a0ee92bdf27c5c60() {
    padding::run("test_the_total_is_divisible_by_sixteen[]", 0);
}
#[test]
fn padding_2f6c56e29722486e() {
    padding::run("test_the_total_is_divisible_by_sixteen[x]", 1);
}
#[test]
fn padding_66c10bf90afb4811() {
    padding::run("test_the_total_is_divisible_by_sixteen[xxxxxxxxxxx]", 2);
}
#[test]
fn padding_76aa4ddaae706a70() {
    padding::run("test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxx]", 3);
}
#[test]
fn padding_634dc231d35f62c8() {
    padding::run("test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxx]", 4);
}
#[test]
fn padding_0dddad9eb79a0953() {
    padding::run(
        "test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxxx]",
        5,
    );
}
#[test]
fn padding_9f72cc3bc62e0842() {
    padding::run(
        "test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxxxx]",
        6,
    );
}
#[test]
fn padding_286001e0dd2a30df() {
    padding::run(
        "test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        7,
    );
}
#[test]
fn padding_70fb80e38532f1df() {
    padding::run(
        "test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        8,
    );
}
#[test]
fn padding_17a6eb0bd6c11dd1() {
    padding::run(
        "test_the_total_is_divisible_by_sixteen[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        9,
    );
}
#[test]
fn padding_41ca8a41692864eb() {
    padding::run("test_the_total_is_divisible_by_sixteen[payload10]", 10);
}
#[test]
fn padding_b768130b5b9860c1() {
    padding::run("test_the_padding_is_at_least_twelve_bytes[]", 11);
}
#[test]
fn padding_be9231d925407d59() {
    padding::run("test_the_padding_is_at_least_twelve_bytes[x]", 12);
}
#[test]
fn padding_28d76491e11c32b5() {
    padding::run("test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxx]", 13);
}
#[test]
fn padding_638bb68c642ff08d() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxx]",
        14,
    );
}
#[test]
fn padding_7640422b510ebb98() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxx]",
        15,
    );
}
#[test]
fn padding_497711df3ac48283() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxxx]",
        16,
    );
}
#[test]
fn padding_23ad93b8194bae65() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxxxx]",
        17,
    );
}
#[test]
fn padding_6ad7718c63bbe1d9() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        18,
    );
}
#[test]
fn padding_97d3f361e245d804() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        19,
    );
}
#[test]
fn padding_10f5ec4456b79494() {
    padding::run(
        "test_the_padding_is_at_least_twelve_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        20,
    );
}
#[test]
fn padding_3fffedecfa7ed34a() {
    padding::run("test_the_padding_is_at_least_twelve_bytes[payload10]", 21);
}
#[test]
fn padding_49bb0e9e48d38ab3() {
    padding::run("test_the_padding_is_at_most_1024_bytes[]", 22);
}
#[test]
fn padding_75eb1fb8b7acbea3() {
    padding::run("test_the_padding_is_at_most_1024_bytes[x]", 23);
}
#[test]
fn padding_fad7ec7952228b11() {
    padding::run("test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxx]", 24);
}
#[test]
fn padding_31dcbaae83643bbc() {
    padding::run("test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxx]", 25);
}
#[test]
fn padding_daf271d4fcfbacd3() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxx]",
        26,
    );
}
#[test]
fn padding_0f63b6e3703fe0b2() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxxx]",
        27,
    );
}
#[test]
fn padding_ad3eb13ed7050cf2() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxxxx]",
        28,
    );
}
#[test]
fn padding_ad9996676e5a6757() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        29,
    );
}
#[test]
fn padding_3858d8e06541fdf2() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        30,
    );
}
#[test]
fn padding_db2596b8ab71ebb4() {
    padding::run(
        "test_the_padding_is_at_most_1024_bytes[xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx]",
        31,
    );
}
#[test]
fn padding_7076aa48e2fef75a() {
    padding::run("test_the_padding_is_at_most_1024_bytes[payload10]", 32);
}
#[test]
fn padding_98a7b7eca04efffc() {
    padding::run("test_the_padding_is_not_constant", 33);
}
#[test]
fn padding_ac3728a688a99d26() {
    padding::run("test_the_payload_is_returned_unchanged_at_the_front", 34);
}
#[test]
fn padding_fd41ce72ad54c5da() {
    padding::run("test_the_padding_source_is_os_urandom", 35);
}
#[test]
fn padding_447ad6445c3e2412() {
    padding::run(
        "test_the_exempt_file_says_it_is_generated_on_its_first_line",
        36,
    );
}
