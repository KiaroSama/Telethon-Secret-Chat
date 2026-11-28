use kiaro_secret_chat::error::Error;
pub fn conversation_context_keeps_useful_identity_without_exposing_metadata() {
    for refusal in [
        Error::FrameRejected,
        Error::MediaKindMismatch {
            file_name: "PRIVATE_FILENAME_SENTINEL".into(),
            kind: "photo",
        },
    ] {
        let error = refusal.with_chat(42);
        for text in [error.to_string(), format!("{error:?}")] {
            assert!(text.contains("42"));
            assert!(!text.contains("PRIVATE_FILENAME_SENTINEL"));
            assert!(text.len() > 20);
        }
        let replaced = error.with_chat(43);
        assert!(replaced.to_string().contains("43"));
        assert!(!replaced.to_string().contains("42"));
    }
}
