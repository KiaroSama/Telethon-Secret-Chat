#![allow(clippy::unwrap_used)]
use super::material::key;
use kiaro_secret_chat::{
    chat::{Chat, DhRecord, Exchange},
    crypto,
    dh::SecretExponent,
    enums,
    protocol::Effects,
    rekey::{self, Collision},
    types,
};
pub const NOW: u64 = 1_800_000_000;
pub fn chat() -> Chat {
    let mut c = Chat::for_role(7, 1, 2, true);
    c.adopt_key(key(5, 3)).unwrap();
    c.rekeyed_at = NOW;
    c
}
fn requested(c: &mut Chat, id: i64) {
    c.exchange = Some(Exchange::Requested {
        id,
        secret: SecretExponent::from_bytes([1; 256]),
    });
}
pub fn run(case: &str) {
    let (name, param) = case
        .split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')));
    let mut c = chat();
    match name {
        "test_a_fresh_chat_does_not_rekey" => assert!(!rekey::should_rekey(&c, NOW)),
        "test_a_hundred_messages_triggers_it" => {
            c.messages_since_rekey = 100;
            assert!(!rekey::should_rekey(&c, NOW));
            c.messages_since_rekey = 101;
            assert!(rekey::should_rekey(&c, NOW));
        }
        "test_a_week_triggers_it" => {
            c.rekeyed_at = NOW - 8 * 24 * 3600;
            c.messages_since_rekey = 1;
            assert!(rekey::should_rekey(&c, NOW));
        }
        "test_an_old_but_unused_key_does_not_trigger_it" => {
            c.rekeyed_at = NOW - 30 * 24 * 3600;
            c.messages_since_rekey = 0;
            assert!(!rekey::should_rekey(&c, NOW));
        }
        "test_an_exchange_already_running_does_not_trigger_another" => {
            c.messages_since_rekey = 500;
            requested(&mut c, 12345);
            assert!(!rekey::should_rekey(&c, NOW));
        }
        "test_a_retained_previous_key_does_not_trigger_another" => {
            c.messages_since_rekey = 500;
            c.previous_key = Some(key(5, 3));
            assert!(!rekey::should_rekey(&c, NOW));
        }
        "test_the_exchange_id_is_a_full_width_random_int64" => {
            let mut ids = std::collections::BTreeSet::new();
            for _ in 0..200 {
                let id = crypto::random_id().unwrap();
                assert_eq!(i64::from_le_bytes(id.to_le_bytes()), id);
                ids.insert(id);
            }
            assert_eq!(ids.len(), 200);
            assert!(ids.iter().any(|x| x.unsigned_abs() > 1 << 40));
        }
        "test_the_exchange_id_does_not_come_from_the_random_module" => {
            struct Calls(Vec<String>);
            impl<'a> syn::visit::Visit<'a> for Calls {
                fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
                    if let syn::Expr::Path(p) = &*c.func {
                        self.0
                            .extend(p.path.segments.iter().map(|s| s.ident.to_string()));
                    }
                    syn::visit::visit_expr_call(self, c);
                }
            }
            let ast = syn::parse_file(include_str!("../../src/rekey.rs")).unwrap();
            let f = ast
                .items
                .iter()
                .find_map(|i| match i {
                    syn::Item::Fn(f) if f.sig.ident == "start" => Some(f),
                    _ => None,
                })
                .unwrap();
            let mut calls = Calls(Vec::new());
            syn::visit::Visit::visit_block(&mut calls, &f.block);
            assert!(calls.0.iter().any(|x| x == "random_id"));
            assert!(
                !calls
                    .0
                    .iter()
                    .any(|x| ["rand", "fastrand", "randint", "getrandbits"].contains(&x.as_str()))
            );
            let ast = syn::parse_file(include_str!("../../src/crypto.rs")).unwrap();
            let mut id = None;
            let mut bytes = None;
            for item in ast.items {
                if let syn::Item::Fn(f) = item {
                    if f.sig.ident == "random_id" {
                        id = Some(f);
                    } else if f.sig.ident == "random_bytes" {
                        bytes = Some(f);
                    }
                }
            }
            let mut call = Calls(Vec::new());
            syn::visit::Visit::visit_block(&mut call, &id.unwrap().block);
            assert!(call.0.iter().any(|x| x == "from_le_bytes"));
            assert!(call.0.iter().any(|x| x == "random_bytes"));
            let mut call = Calls(Vec::new());
            syn::visit::Visit::visit_block(&mut call, &bytes.unwrap().block);
            assert!(call.0.windows(2).any(|x| x == ["getrandom", "fill"]));
            let _ = crypto::random_id().unwrap();
        }
        "test_the_documented_tie_break" => {
            let v: Vec<_> = param.split('-').collect();
            let expected = match v[2] {
                "abandon_theirs" => Collision::AbandonTheirs,
                "join_theirs" => Collision::JoinTheirs,
                "abort_both" => Collision::AbortBoth,
                _ => panic!("unknown collision fixture"),
            };
            assert_eq!(
                rekey::resolve_collision(v[0].parse().unwrap(), v[1].parse().unwrap()),
                expected
            );
            assert_eq!(rekey::resolve_collision(-1, 1), Collision::JoinTheirs);
            assert_eq!(rekey::resolve_collision(1, -1), Collision::AbandonTheirs);
            assert_eq!(rekey::resolve_collision(1, 1), Collision::AbortBoth);
        }
        "test_an_exchange_may_be_aborted_before_a_commitment" => {
            assert!(!rekey::may_abort(&c));
            requested(&mut c, 1);
            assert!(rekey::may_abort(&c));
        }
        "test_an_exchange_may_not_be_aborted_after_one" => {
            if param == "accepted" {
                c.exchange = Some(Exchange::Accepted {
                    id: 1,
                    key: key(9, 1),
                });
            } else {
                requested(&mut c, 1);
                rekey::adopt_new_key(&mut c, key(9, 1), NOW).unwrap();
            }
            assert!(!rekey::may_abort(&c));
        }
        "test_a_frame_is_matched_to_whichever_held_key_it_names" => {
            c.previous_key = Some(key(9, 1));
            assert_eq!(
                rekey::select_key(&c, key(5, 3).fingerprint())
                    .unwrap()
                    .expose(),
                key(5, 3).expose()
            );
            assert_eq!(
                rekey::select_key(&c, key(9, 1).fingerprint())
                    .unwrap()
                    .expose(),
                key(9, 1).expose()
            );
            assert!(rekey::select_key(&c, 999).is_none());
        }
        "test_the_pending_key_can_decrypt_too" => {
            c.exchange = Some(Exchange::Accepted {
                id: 5,
                key: key(3, 7),
            });
            assert_eq!(
                rekey::select_key(&c, key(3, 7).fingerprint())
                    .unwrap()
                    .expose(),
                key(3, 7).expose()
            );
        }
        "test_the_counters_are_not_reset_by_a_rekey" => {
            c.in_seq_no = 17;
            c.out_seq_no = 23;
            rekey::adopt_new_key(&mut c, key(11, 0), NOW).unwrap();
            assert_eq!((c.in_seq_no, c.out_seq_no), (17, 23));
        }
        "test_adopting_a_new_key_retains_the_old_one" => {
            rekey::adopt_new_key(&mut c, key(11, 0), NOW).unwrap();
            assert_eq!(c.key.as_ref().unwrap().expose(), key(11, 0).expose());
            assert_eq!(
                c.previous_key.as_ref().unwrap().expose(),
                key(5, 3).expose()
            );
        }
        "test_the_previous_key_is_dropped_once_the_new_one_has_carried_a_message" => {
            rekey::adopt_new_key(&mut c, key(11, 0), NOW).unwrap();
            rekey::observe_key(&mut c, key(11, 0).fingerprint(), NOW).unwrap();
            rekey::retire_if_settled(&mut c);
            assert!(c.previous_key.is_none());
        }
        "test_the_old_key_is_kept_while_a_gap_is_open" => {
            rekey::adopt_new_key(&mut c, key(11, 0), NOW).unwrap();
            c.gap_end = Some(1);
            rekey::retire_if_settled(&mut c);
            assert!(c.previous_key.is_some());
            c.gap_end = None;
            rekey::retire_if_settled(&mut c);
            assert!(c.previous_key.is_some());
            rekey::observe_key(&mut c, key(11, 0).fingerprint(), NOW).unwrap();
            rekey::retire_if_settled(&mut c);
            assert!(c.previous_key.is_none());
            rekey::adopt_new_key(&mut c, key(12, 0), NOW).unwrap();
            c.gap_end = Some(1);
            rekey::observe_key(&mut c, key(12, 0).fingerprint(), NOW).unwrap();
            rekey::retire_if_settled(&mut c);
            assert!(
                c.previous_key.is_some(),
                "a confirmed key still needs the old key while its gap is open"
            );
            c.gap_end = None;
            rekey::retire_if_settled(&mut c);
            assert!(c.previous_key.is_none());
        }
        "test_actions_for_a_foreign_exchange_are_ignored_and_not_applied" => {
            let a: enums::DecryptedMessageAction = match param {
                "action0" => types::DecryptedMessageActionAcceptKey {
                    exchange_id: 12345,
                    g_b: vec![2; 256],
                    key_fingerprint: 1,
                }
                .into(),
                "action1" => types::DecryptedMessageActionCommitKey {
                    exchange_id: 12345,
                    key_fingerprint: 1,
                }
                .into(),
                "action2" => types::DecryptedMessageActionAbortKey { exchange_id: 12345 }.into(),
                _ => panic!("unknown foreign fixture"),
            };
            let before = serde_json::to_value(&c).unwrap();
            let mut effects = Effects::default();
            assert!(!rekey::handle(&mut c, &a, &mut effects, NOW).unwrap());
            assert_eq!(serde_json::to_value(&c).unwrap(), before);
            assert!(effects.sends.is_empty() && effects.events.is_empty());
        }
        "test_a_request_key_while_the_previous_key_is_held_is_answered_with_abort" => {
            c.previous_key = Some(key(9, 1));
            c.dh = Some(DhRecord {
                g: 2,
                prime: super::dh_material::prime().to_bytes_be(),
            });
            let a = types::DecryptedMessageActionRequestKey {
                exchange_id: 77,
                g_a: (num_bigint::BigUint::from(1u32) << 2047usize).to_bytes_be(),
            }
            .into();
            let mut effects = Effects::default();
            rekey::handle(&mut c, &a, &mut effects, NOW).unwrap();
            assert!(c.outbox.values().any(|row|matches!(kiaro_secret_chat::framing::unwrap(&row.body).unwrap().message,enums::DecryptedMessage::Service(s)if matches!(&s.action,enums::DecryptedMessageAction::AbortKey(a)if a.exchange_id==77))));
            assert!(
                effects
                    .sends
                    .iter()
                    .any(|s| s.outgoing.body == c.outbox[&s.outgoing.seq_no].body)
            );
        }
        _ => panic!("unknown rekey core scenario {case}"),
    }
}
