#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{Serializable, enums, framing, types};
fn message() -> enums::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1234,
        ttl: 0,
        message: "hello".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}
fn wrapped() -> types::DecryptedMessageLayer {
    let enums::DecryptedMessageLayer::Layer(w) = framing::wrap(message(), 144, 2, 3).unwrap();
    w
}
pub fn run(case: &str) {
    let (name, param) = case
        .split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')));
    match name {
        "test_a_message_is_wrapped_in_decrypted_message_layer" => {
            let enums::DecryptedMessageLayer::Layer(w) =
                framing::wrap(message(), 144, 4, 7).unwrap();
            assert_eq!((w.layer, w.in_seq_no, w.out_seq_no), (144, 4, 7));
        }
        "test_a_service_message_is_wrapped_too" => {
            let service = types::DecryptedMessageService {
                random_id: 9,
                action: enums::DecryptedMessageAction::Noop,
            }
            .into();
            let enums::DecryptedMessageLayer::Layer(w) = framing::wrap(service, 73, 0, 0).unwrap();
            assert!(matches!(w.message, enums::DecryptedMessage::Service(_)));
        }
        "test_a_wrapped_message_survives_serialization" => {
            let w: enums::DecryptedMessageLayer = wrapped().into();
            let b = framing::unwrap(&w.to_bytes()).unwrap();
            assert_eq!(b.out_seq_no, 3);
            let mut trailing = w.to_bytes();
            trailing.push(0);
            assert!(framing::unwrap(&trailing).is_err());
            assert!(matches!(b.message,enums::DecryptedMessage::Message(m)if m.message=="hello"));
        }
        "test_random_bytes_is_thirty_one_bytes" => assert_eq!(wrapped().random_bytes.len(), 31),
        "test_random_bytes_differs_every_time" => {
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..30 {
                seen.insert(wrapped().random_bytes);
            }
            assert_eq!(seen.len(), 30);
        }
        "test_a_wrapper_below_layer_46_is_refused" => {
            let mut w = wrapped();
            w.layer = 45;
            let e: enums::DecryptedMessageLayer = w.into();
            assert!(framing::unwrap(&e.to_bytes()).is_err());
        }
        "test_a_body_that_is_not_a_wrapper_is_refused" => {
            assert!(framing::unwrap(&message().to_bytes()).is_err());
            assert!(framing::unwrap(&enums::DecryptedMessageAction::Noop.to_bytes()).is_err());
        }
        "test_an_unparseable_body_is_refused_not_guessed" => {
            assert!(framing::unwrap(&[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]).is_err())
        }
        "test_the_remote_layer_starts_at_46" => assert_eq!(framing::INITIAL_REMOTE_LAYER, 46),
        "test_the_remote_layer_only_ever_rises" => {
            assert_eq!(framing::raise_remote_layer(46, 101), 101);
            assert_eq!(framing::raise_remote_layer(101, 46), 101);
            assert_eq!(framing::raise_remote_layer(101, 101), 101);
            assert_eq!(framing::raise_remote_layer(144, 73), 144);
        }
        "test_the_outgoing_layer_is_clamped_between_73_and_144" => {
            let (a, b) = param.split_once('-').unwrap();
            assert_eq!(
                framing::outgoing_layer(a.parse().unwrap()),
                b.parse::<i32>().unwrap()
            );
            assert_eq!(framing::outgoing_layer(1000), 144);
        }
        "test_a_peer_that_cannot_reach_73_is_refused" => {
            assert!(framing::require_supported_layer(72).is_err());
            framing::require_supported_layer(73).unwrap();
        }
        "test_the_seq_no_transform" => {
            let v: Vec<_> = param.split('-').collect();
            let outbound = v[0] == "True";
            let n: Vec<i32> = v[1..].iter().map(|x| x.parse().unwrap()).collect();
            assert_eq!(framing::transform_in_seq_no(n[0], outbound).unwrap(), n[2]);
            assert_eq!(framing::transform_out_seq_no(n[1], outbound).unwrap(), n[3]);
        }
        "test_the_transform_is_reversible" => {
            for outbound in [true, false] {
                for raw in 0..10 {
                    assert_eq!(
                        framing::raw_seq_no(framing::transform_out_seq_no(raw, outbound).unwrap())
                            .unwrap(),
                        raw
                    );
                }
            }
            for raw in [0, 1, 100, 1000] {
                assert_eq!(framing::transform_in_seq_no(raw, true).unwrap(), 2 * raw);
                assert_eq!(
                    framing::transform_out_seq_no(raw, true).unwrap(),
                    2 * raw + 1
                );
                assert_eq!(
                    framing::transform_in_seq_no(raw, false).unwrap(),
                    2 * raw + 1
                );
                assert_eq!(framing::transform_out_seq_no(raw, false).unwrap(), 2 * raw);
            }
            assert!(framing::transform_out_seq_no(i32::MAX, true).is_err());
            assert!(framing::transform_in_seq_no(-1, false).is_err());
            assert!(framing::raw_seq_no(-1).is_err());
        }
        _ => panic!("unknown framing scenario {case}"),
    }
}
