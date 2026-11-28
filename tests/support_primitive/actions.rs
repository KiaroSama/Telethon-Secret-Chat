#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    Deserializable, Serializable,
    actions::{self, ActionGroup},
    enums, types,
};
#[allow(dead_code)] // Wire/classification target shares fixtures with callback target.
pub const NAMES: [&str; 13] = [
    "decryptedMessageActionSetMessageTTL",
    "decryptedMessageActionReadMessages",
    "decryptedMessageActionDeleteMessages",
    "decryptedMessageActionScreenshotMessages",
    "decryptedMessageActionFlushHistory",
    "decryptedMessageActionResend",
    "decryptedMessageActionNotifyLayer",
    "decryptedMessageActionTyping",
    "decryptedMessageActionRequestKey",
    "decryptedMessageActionAcceptKey",
    "decryptedMessageActionCommitKey",
    "decryptedMessageActionAbortKey",
    "decryptedMessageActionNoop",
];
pub fn action(name: &str, group_values: bool) -> enums::DecryptedMessageAction {
    let exchange_id = if group_values { 1 } else { 9 };
    let ids = |v| if group_values { vec![] } else { v };
    match name {
        "decryptedMessageActionSetMessageTTL" => types::DecryptedMessageActionSetMessageTtl {
            ttl_seconds: if group_values { 1 } else { 60 },
        }
        .into(),
        "decryptedMessageActionReadMessages" => types::DecryptedMessageActionReadMessages {
            random_ids: ids(vec![1, 2]),
        }
        .into(),
        "decryptedMessageActionDeleteMessages" => types::DecryptedMessageActionDeleteMessages {
            random_ids: ids(vec![3]),
        }
        .into(),
        "decryptedMessageActionScreenshotMessages" => {
            types::DecryptedMessageActionScreenshotMessages {
                random_ids: ids(vec![4]),
            }
            .into()
        }
        "decryptedMessageActionFlushHistory" => enums::DecryptedMessageAction::FlushHistory,
        "decryptedMessageActionResend" => types::DecryptedMessageActionResend {
            start_seq_no: 1,
            end_seq_no: if group_values { 1 } else { 3 },
        }
        .into(),
        "decryptedMessageActionNotifyLayer" => {
            types::DecryptedMessageActionNotifyLayer { layer: 144 }.into()
        }
        "decryptedMessageActionTyping" => types::DecryptedMessageActionTyping {
            action: types::SendMessageTypingAction {}.into(),
        }
        .into(),
        "decryptedMessageActionRequestKey" => types::DecryptedMessageActionRequestKey {
            exchange_id,
            g_a: if group_values { vec![] } else { vec![1; 256] },
        }
        .into(),
        "decryptedMessageActionAcceptKey" => types::DecryptedMessageActionAcceptKey {
            exchange_id,
            g_b: if group_values { vec![] } else { vec![2; 256] },
            key_fingerprint: if group_values { 0 } else { 5 },
        }
        .into(),
        "decryptedMessageActionCommitKey" => types::DecryptedMessageActionCommitKey {
            exchange_id,
            key_fingerprint: if group_values { 0 } else { 5 },
        }
        .into(),
        "decryptedMessageActionAbortKey" => {
            types::DecryptedMessageActionAbortKey { exchange_id }.into()
        }
        "decryptedMessageActionNoop" => enums::DecryptedMessageAction::Noop,
        _ => panic!("unknown fixture action {name}"),
    }
}
#[allow(dead_code)] // Only the scalar target dispatches these scenarios.
pub fn run(case: &str) {
    let (name, param) = case
        .split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')));
    match name {
        "test_there_are_thirteen_and_the_test_covers_them_all" => {
            let expected: std::collections::BTreeSet<_> =
                include_str!("../../schema/end-to-end.tl")
                    .lines()
                    .filter(|l| l.trim_end().ends_with(" = DecryptedMessageAction;"))
                    .map(|l| l.split('#').next().unwrap())
                    .collect();
            assert_eq!(expected, NAMES.into_iter().collect());
            assert_eq!(NAMES.len(), 13);
            for name in NAMES {
                let a = action(name, false);
                let id = u32::from_le_bytes(a.to_bytes()[..4].try_into().unwrap());
                assert_eq!(kiaro_secret_chat::name_for_id(id), name);
            }
        }
        "test_every_action_round_trips_on_the_wire" => {
            let a = action(param, false);
            let expected = a.to_bytes();
            let service: enums::DecryptedMessage = types::DecryptedMessageService {
                random_id: 1,
                action: a,
            }
            .into();
            let bytes = service.to_bytes();
            kiaro_secret_chat::wire::validate(&bytes, "DecryptedMessage").unwrap();
            let enums::DecryptedMessage::Service(s) =
                enums::DecryptedMessage::from_bytes(&bytes).unwrap()
            else {
                panic!("wrong service family")
            };
            assert_eq!(s.action.to_bytes(), expected);
        }
        "test_every_action_is_classified" => assert!(matches!(
            actions::group_of(&action(param, false)),
            ActionGroup::Conversation | ActionGroup::Protocol | ActionGroup::Rekey
        )),
        "test_the_groups_are_as_the_data_model_states" => {
            let (action_name, group) = param.rsplit_once('-').unwrap();
            assert_eq!(
                actions::group_of(&action(action_name, true)).as_str(),
                group
            );
        }
        _ => panic!("unknown action scalar scenario {case}"),
    }
}
