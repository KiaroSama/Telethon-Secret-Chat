#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto::{self, Direction, SharedKey};
use proptest::prelude::*;
pub fn check() {
    let key = SharedKey::from_slice(&(0..=255).collect::<Vec<u8>>()).unwrap();
    for direction in [Direction::Initiator, Direction::Responder] {
        let frame = crypto::encrypt_frame(&key, b"hello", direction).unwrap();
        assert_eq!(
            &*crypto::decrypt_frame(&key, &frame, direction).unwrap(),
            b"hello"
        );
    }
    let frame = crypto::encrypt_frame(&key, b"payload", Direction::Initiator).unwrap();
    assert!(crypto::decrypt_frame(&key, &frame, Direction::Responder).is_err());
    for i in 0..frame.len() {
        let mut corrupt = frame.clone();
        corrupt[i] ^= 1;
        assert!(crypto::decrypt_frame(&key, &corrupt, Direction::Initiator).is_err());
    }
    for end in 0..frame.len() {
        assert!(crypto::decrypt_frame(&key, &frame[..end], Direction::Initiator).is_err());
    }
    let config = proptest::test_runner::Config {
        source_file: Some(file!()),
        test_name: Some("native_arbitrary_bodies_roundtrip"),
        ..Default::default()
    };
    let mut runner = proptest::test_runner::TestRunner::new(config);
    runner
        .run(
            &(
                proptest::collection::vec(any::<u8>(), 0..8192),
                any::<bool>(),
            ),
            |(body, responder)| {
                let direction = if responder {
                    Direction::Responder
                } else {
                    Direction::Initiator
                };
                let frame = crypto::encrypt_frame(&key, &body, direction).unwrap();
                prop_assert_eq!(
                    &*crypto::decrypt_frame(&key, &frame, direction).unwrap(),
                    &body
                );
                Ok(())
            },
        )
        .unwrap();
}
