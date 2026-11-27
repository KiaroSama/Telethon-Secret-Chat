use kiaro_secret_chat::export::serialize_json_string;
use kiaro_secret_chat::ported::tdexport_json_serialize::serialize_bytes;

#[test]
fn json_string_roundtrips_every_control_and_unicode_separator() {
    for text in [
        (0u8..=31).map(char::from).collect::<String>(),
        "before\u{2028}middle\u{2029}after".into(),
        "فارسی 日本語 👋 \\\"".into(),
    ] {
        let encoded =
            serialize_json_string(&text).expect("serialization must accept valid Rust strings");
        let decoded: String = serde_json::from_str(&encoded).expect("export must be valid JSON");
        assert_eq!(decoded, text);
    }
}

#[test]
fn binary_string_serialization_refuses_invalid_utf8_instead_of_emitting_broken_json() {
    assert!(serialize_bytes(vec![0xff, 0xfe]).is_err());
    let valid = serialize_bytes("سلام".as_bytes().to_vec()).unwrap();
    assert_eq!(serde_json::from_slice::<String>(&valid).unwrap(), "سلام");
}
