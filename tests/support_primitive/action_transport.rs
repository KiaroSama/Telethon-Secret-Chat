#![allow(clippy::unwrap_used)]
use super::{action_cases, support};
use kiaro_secret_chat::{
    Serializable,
    chat::Chat,
    crypto, enums, framing,
    manager::{Event, Options, SecretChatManager},
    storage::State,
    types,
};
use std::sync::{Arc, Mutex};
type Manager = SecretChatManager<support::TestStorage, support::Fake>;
async fn pair() -> (
    Manager,
    Manager,
    support::TestStorage,
    support::TestStorage,
    Arc<Mutex<support::World>>,
    i32,
) {
    let (aa, bb) = support::Fake::pair();
    let world = aa.world.clone();
    let sa = support::TestStorage::default();
    let sb = support::TestStorage::default();
    let a = SecretChatManager::new(aa, sa.clone(), "11".repeat(32), Options::default()).unwrap();
    let b = SecretChatManager::new(bb, sb.clone(), "22".repeat(32), Options::default()).unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(support::peer(2000)).await.unwrap().id;
    support::pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    support::pump(&a, &b, &world).await;
    (a, b, sa, sb, world, id)
}
fn stored(storage: &support::TestStorage) -> State {
    storage.state.lock().unwrap().as_ref().unwrap().clone()
}
fn chat(storage: &support::TestStorage, id: i32) -> Chat {
    stored(storage).chats.remove(&id).unwrap()
}
fn service_log(manager: &Manager) -> Arc<Mutex<Vec<String>>> {
    let rows = Arc::new(Mutex::new(Vec::new()));
    let sink = rows.clone();
    manager
        .on("ServiceActionReceived", "primitive_actions", move |e| {
            if let Event::ServiceActionReceived { action_name, .. } = e {
                sink.lock().unwrap().push(action_name);
            }
            Ok(())
        })
        .unwrap();
    rows
}
fn update(id: i32, bytes: Vec<u8>) -> grammers_tl_types::enums::Update {
    grammers_tl_types::types::UpdateNewEncryptedMessage {
        qts: 1,
        message: grammers_tl_types::types::EncryptedMessageService {
            random_id: 1,
            chat_id: id,
            date: 1,
            bytes,
        }
        .into(),
    }
    .into()
}
pub async fn run(case: &str) {
    let (name, param) = case
        .split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')));
    let (a, b, sa, sb, world, id) = pair().await;
    match name {
        "test_the_peers_ttl_is_stored" => {
            b.set_ttl(id, 300).await.unwrap();
            support::pump(&a, &b, &world).await;
            assert_eq!(a.status(id).unwrap().ttl, 300);
        }
        "test_a_ttl_of_zero_disables_it" => {
            b.set_ttl(id, 300).await.unwrap();
            support::pump(&a, &b, &world).await;
            b.set_ttl(id, 0).await.unwrap();
            support::pump(&a, &b, &world).await;
            assert_eq!(a.status(id).unwrap().ttl, 0);
        }
        "test_every_action_reaches_the_application" => {
            let action_name = if param == "decryptedMessageActionResend" {
                "decryptedMessageActionNoop"
            } else {
                param
            };
            let seen = service_log(&a);
            let c = chat(&sb, id);
            let service: enums::DecryptedMessage = types::DecryptedMessageService {
                random_id: 1,
                action: action_cases::action(action_name, false),
            }
            .into();
            let wrapper = framing::wrap(
                service,
                144,
                framing::transform_in_seq_no(c.in_seq_no, c.is_outbound).unwrap(),
                framing::transform_out_seq_no(c.out_seq_no, c.is_outbound).unwrap(),
            )
            .unwrap();
            let frame = crypto::encrypt_frame(
                c.key.as_ref().unwrap(),
                &wrapper.to_bytes(),
                c.out_direction(),
            )
            .unwrap();
            a.handle_update(&update(id, frame)).await;
            let mut expected = action_name.to_owned();
            expected[..1].make_ascii_uppercase();
            assert_eq!(*seen.lock().unwrap(), [expected]);
        }
        "test_an_unknown_action_is_reported_rather_than_dropped" => {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = seen.clone();
            a.on("DecryptFailed", "primitive_unknown", move |e| {
                if let Event::DecryptFailed { reason, .. } = e {
                    sink.lock().unwrap().push(reason);
                }
                Ok(())
            })
            .unwrap();
            let c = chat(&sb, id);
            let frame = crypto::encrypt_frame(
                c.key.as_ref().unwrap(),
                &[0xde, 0xad, 0xbe, 0xef].repeat(4),
                c.out_direction(),
            )
            .unwrap();
            a.handle_update(&update(id, frame)).await;
            let rows = seen.lock().unwrap();
            assert!(!rows.is_empty());
            assert!(rows[0].contains("constructor") || rows[0].contains("parsed"));
        }
        "test_the_six_the_consumer_needs_all_have_a_send_path" => {
            let seen = service_log(&b);
            a.set_ttl(id, 30).await.unwrap();
            a.mark_read(id, vec![11, 12]).await.unwrap();
            a.delete_messages(id, vec![13]).await.unwrap();
            a.screenshot(id, vec![14]).await.unwrap();
            a.flush_history(id).await.unwrap();
            a.set_typing(id, None).await.unwrap();
            support::pump(&a, &b, &world).await;
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    "DecryptedMessageActionSetMessageTTL",
                    "DecryptedMessageActionReadMessages",
                    "DecryptedMessageActionDeleteMessages",
                    "DecryptedMessageActionScreenshotMessages",
                    "DecryptedMessageActionFlushHistory",
                    "DecryptedMessageActionTyping"
                ]
            );
        }
        "test_setting_a_ttl_stores_it_on_this_side_too" => {
            a.set_ttl(id, 45).await.unwrap();
            assert_eq!(a.status(id).unwrap().ttl, 45);
        }
        "test_a_message_sent_under_a_ttl_carries_it" => {
            a.set_ttl(id, 45).await.unwrap();
            a.send_message(id, "this one self-destructs", None, None)
                .await
                .unwrap();
            support::pump(&a, &b, &world).await;
            assert_eq!(b.read_history(id, 1).unwrap()[0].ttl, 45);
        }
        "test_a_service_action_advances_the_counter" => {
            let before = chat(&sa, id).out_seq_no;
            a.set_ttl(id, 10).await.unwrap();
            assert_eq!(chat(&sa, id).out_seq_no, before + 1);
        }
        "test_deleting_an_unacknowledged_message_leaves_no_hole" => {
            let random = a.send_message(id, "regrettable", None, None).await.unwrap();
            let before = chat(&sa, id).outbox;
            a.delete_messages(id, vec![random]).await.unwrap();
            let after = chat(&sa, id).outbox;
            for sequence in before.keys() {
                assert!(after.contains_key(sequence));
            }
            let sequence = before
                .iter()
                .filter(|(_, v)| !v.service)
                .map(|(k, _)| *k)
                .max()
                .unwrap();
            let rewritten = &after[&sequence];
            assert!(!rewritten.body.windows(11).any(|w| w == b"regrettable"));
            assert_eq!(rewritten.seq_no, sequence);
            let wrapper = framing::unwrap(&rewritten.body).unwrap();
            assert!(
                matches!(wrapper.message,enums::DecryptedMessage::Service(s)if matches!(&s.action,enums::DecryptedMessageAction::DeleteMessages(a)if a.random_ids==[random]))
            );
        }
        _ => panic!("unknown action transport scenario {case}"),
    }
}
