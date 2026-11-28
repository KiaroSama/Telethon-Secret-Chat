#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{Serializable, enums, types};
use proptest::prelude::*;
pub fn check() {
    let config = proptest::test_runner::Config {
        source_file: Some(file!()),
        test_name: Some("native_arbitrary_wire_input_never_panics"),
        ..Default::default()
    };
    let mut runner = proptest::test_runner::TestRunner::new(config);
    runner
        .run(&proptest::collection::vec(any::<u8>(), 0..4096), |bytes| {
            let _ = kiaro_secret_chat::framing::unwrap(&bytes);
            Ok(())
        })
        .unwrap();
    let structured = (".{0,80}", 0usize..7, any::<u32>(), any::<usize>()).prop_map(
        |(text, mutation, flags, cut)| {
            let length = text.encode_utf16().count() as i32;
            let message: enums::DecryptedMessage = types::DecryptedMessage {
                no_webpage: false,
                silent: false,
                random_id: 17,
                ttl: 0,
                message: text,
                media: None,
                entities: Some(vec![types::MessageEntityBold { offset: 0, length }.into()]),
                via_bot_name: None,
                reply_to_random_id: None,
                grouped_id: None,
            }
            .into();
            let mut bytes = enums::DecryptedMessageLayer::from(types::DecryptedMessageLayer {
                random_bytes: vec![1; 31],
                layer: 144,
                in_seq_no: 1,
                out_seq_no: 0,
                message,
            })
            .to_bytes();
            match mutation {
                0 => {}
                1 => bytes[..4].fill(0xff),
                2 => bytes[4] = 254,
                3 => bytes[48..52].fill(0xff),
                4 => bytes[52..56].copy_from_slice(&flags.to_le_bytes()),
                5 => bytes.truncate(cut % (bytes.len() + 1)),
                _ => {
                    let marker = 0x1cb5c415u32.to_le_bytes();
                    let at = bytes
                        .windows(4)
                        .rposition(|window| window == marker)
                        .unwrap();
                    bytes[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
                }
            }
            (bytes, mutation)
        },
    );
    runner
        .run(&structured, |(bytes, mutation)| {
            let parsed = kiaro_secret_chat::framing::unwrap(&bytes);
            if mutation == 0 {
                prop_assert!(
                    parsed.is_ok(),
                    "valid wrappers must reach their actual nested fields"
                );
            }
            Ok(())
        })
        .unwrap();
}
