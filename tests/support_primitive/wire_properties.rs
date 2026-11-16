#![allow(clippy::unwrap_used)]
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
}
