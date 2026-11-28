#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    chat::{Chat, ChatState, Exchange},
    crypto::SharedKey,
    error::{CloseReason, Error},
};
use serde_json::Value;
fn key(mult: u8, add: u8) -> [u8; 256] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(mult).wrapping_add(add))
}
fn state(name: &str) -> ChatState {
    match name {
        "requested" => ChatState::Requested,
        "pending" => ChatState::Waiting,
        "ready" => ChatState::Ready,
        "rekeying" => ChatState::Rekeying,
        "closed" => ChatState::Closed,
        _ => panic!("unknown fixture state"),
    }
}
fn chat(initial: ChatState) -> Chat {
    let mut c = Chat::for_role(101, 2222, 333, true);
    if matches!(initial, ChatState::Ready | ChatState::Rekeying) {
        c.adopt_key_bytes(&key(5, 3)).unwrap();
        if initial == ChatState::Rekeying {
            c.transition_to(initial).unwrap();
        }
    } else {
        c.state = initial;
    }
    c
}
pub fn run(case: &str) {
    let (name, param) = case
        .split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')));
    match name {
        "test_an_outbound_chat_starts_requested" => {
            assert_eq!(Chat::for_role(1, 2, 3, true).state, ChatState::Requested)
        }
        "test_an_inbound_chat_starts_pending" => {
            assert_eq!(Chat::for_role(1, 2, 3, false).state, ChatState::Waiting)
        }
        "test_the_legal_transitions_are_allowed" => {
            let (a, b) = param.split_once('-').unwrap();
            let mut c = chat(state(a));
            c.transition_to(state(b)).unwrap();
            assert_eq!(c.state, state(b));
        }
        "test_the_illegal_transitions_are_refused" => {
            let (a, b) = param.split_once('-').unwrap();
            let mut c = chat(state(a));
            assert!(c.transition_to(state(b)).is_err());
            assert_eq!(c.state, state(a));
        }
        "test_closed_is_terminal" => {
            let mut c = chat(ChatState::Closed);
            assert!(matches!(
                c.transition_to(state(param)),
                Err(Error::ChatEnded { .. })
            ));
            assert_eq!(c.state, ChatState::Closed);
            assert!(c.adopt_key_bytes(&key(5, 3)).is_err());
            assert!(c.key.is_none());
        }
        "test_send_works_in_ready_and_in_rekeying" => {
            chat(state(param)).require_sendable().unwrap()
        }
        "test_send_refuses_before_the_key_exists" => {
            let e = chat(state(param)).require_sendable().unwrap_err();
            assert!(matches!(e, Error::ChatNotSendable { .. }));
            assert!(e.to_string().contains(param));
        }
        "test_send_refuses_on_a_closed_chat_with_a_stated_reason" => {
            let mut c = chat(ChatState::Ready);
            c.previous_key = Some(SharedKey::from_slice(&key(9, 1)).unwrap());
            c.close_with_reason(CloseReason::PeerDiscarded);
            assert!(c.key.is_none() && c.previous_key.is_none());
            c.validate().unwrap();
            let e = c.require_sendable().unwrap_err();
            assert!(matches!(e, Error::ChatEnded { .. }));
            assert!(e.to_string().contains("the peer discarded the chat"));
        }
        "test_adopting_a_key_sets_its_fingerprint_in_the_same_call" => {
            let mut c = Chat::for_role(1, 2, 3, true);
            c.adopt_key_bytes(&key(5, 3)).unwrap();
            assert_eq!(c.key.as_ref().unwrap().expose(), &key(5, 3));
            assert_eq!(
                c.snapshot().key_fingerprint,
                Some(kiaro_secret_chat::crypto::key_fingerprint(&key(5, 3)))
            );
            assert_eq!(c.state, ChatState::Ready);
        }
        "test_a_key_of_the_wrong_length_is_refused" => {
            let mut c = Chat::for_role(1, 2, 3, true);
            for length in [0, 1, 32, 255, 257, 1024] {
                assert!(matches!(
                    c.adopt_key_bytes(&vec![0; length]),
                    Err(Error::KeyLength)
                ));
            }
            assert!(c.key.is_none());
            assert_eq!(c.state, ChatState::Requested);
        }
        "test_the_key_and_fingerprint_cannot_drift_apart" => {
            let mut c = chat(ChatState::Ready);
            c.adopt_key_bytes(&key(9, 1)).unwrap();
            assert_eq!(
                c.snapshot().key_fingerprint,
                Some(kiaro_secret_chat::crypto::key_fingerprint(&key(9, 1)))
            );
            assert_eq!(c.key.as_ref().unwrap().expose(), &key(9, 1));
        }
        "test_the_crypto_x_follows_the_side" => {
            let a = Chat::for_role(1, 2, 3, true);
            let b = Chat::for_role(1, 2, 3, false);
            assert_eq!(
                (a.out_direction().offset(), a.in_direction().offset()),
                (0, 8)
            );
            assert_eq!(
                (b.out_direction().offset(), b.in_direction().offset()),
                (8, 0)
            );
        }
        "test_a_chat_round_trips_through_its_record" => {
            let mut c = chat(ChatState::Ready);
            c.ttl = 30;
            c.capability_layer = 143;
            c.in_seq_no = 4;
            c.out_seq_no = 6;
            c.exchange = Some(Exchange::Accepted {
                id: 7788,
                key: SharedKey::from_slice(&key(9, 1)).unwrap(),
            });
            let v = serde_json::to_value(&c).unwrap();
            let back: Chat = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(serde_json::to_value(&back).unwrap(), v);
            assert_eq!(back.key.as_ref().unwrap().expose(), &key(5, 3));
            assert_eq!(
                back.snapshot().key_fingerprint,
                c.snapshot().key_fingerprint
            );
            assert_eq!(back.state, ChatState::Ready);
            match back.exchange.unwrap() {
                Exchange::Accepted { id, key: pending } => {
                    assert_eq!(id, 7788);
                    assert_eq!(pending.expose(), &key(9, 1));
                }
                _ => panic!("pending exchange lost"),
            }
        }
        "test_the_record_carries_every_field_the_data_model_lists" => {
            let c = chat(ChatState::Ready);
            let v = serde_json::to_value(&c).unwrap();
            for field in [
                "id",
                "access_hash",
                "peer_user_id",
                "is_outbound",
                "state",
                "key",
                "exchange",
                "in_seq_no",
                "out_seq_no",
                "capability_layer",
                "ttl",
                "created_at",
                "rekeyed_at",
                "messages_since_rekey",
                "admin_id",
                "participant_id",
            ] {
                assert!(v.get(field).is_some(), "missing persisted field {field}");
            }
            let source = syn::parse_file(include_str!("../../src/chat.rs")).unwrap();
            let fields = source
                .items
                .iter()
                .find_map(|i| match i {
                    syn::Item::Struct(s) if s.ident == "Chat" => Some(&s.fields),
                    _ => None,
                })
                .unwrap();
            let object: &serde_json::Map<String, Value> = v.as_object().unwrap();
            assert_eq!(
                fields.len(),
                object.len(),
                "every model field must serialize"
            );
            for f in fields {
                assert!(object.contains_key(&f.ident.as_ref().unwrap().to_string()));
            }
            assert!(c.snapshot().key_fingerprint.is_some());
        }
        "test_a_chat_never_prints_its_key" => {
            let mut c = chat(ChatState::Ready);
            c.exchange = Some(Exchange::Accepted {
                id: 1,
                key: SharedKey::from_slice(&key(9, 1)).unwrap(),
            });
            let display = format!("{c:?}");
            assert!(
                !display
                    .to_lowercase()
                    .contains(&hex::encode(key(5, 3))[..32])
            );
            assert!(
                !display
                    .to_lowercase()
                    .contains(&hex::encode(key(9, 1))[..32])
            );
            for secret in [key(5, 3), key(9, 1)] {
                assert!(!display.contains(&format!("{secret:?}")));
            }
        }
        _ => panic!("unknown chat scenario {case}"),
    }
}
