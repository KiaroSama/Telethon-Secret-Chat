#![allow(clippy::unwrap_used)]
use super::{material, support};
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, Exchange},
    crypto, enums,
    error::{CloseReason, Error},
    framing,
    manager::{Event, MessageReceived, Options, RpcFailure, SecretChatManager},
    storage::{FileStorage, MemoryStorage, State, Storage},
    types,
};
use std::sync::{Arc, Mutex};
type Manager = SecretChatManager<support::TestStorage, support::Fake>;
struct Pair {
    a: Manager,
    b: Manager,
    sa: support::TestStorage,
    sb: support::TestStorage,
    ta: support::Fake,
    world: Arc<Mutex<support::World>>,
    id: i32,
}
fn state(s: &support::TestStorage) -> State {
    s.state.lock().unwrap().as_ref().unwrap().clone()
}
fn chat(s: &support::TestStorage, id: i32) -> Chat {
    state(s).chats.remove(&id).unwrap()
}
async fn pair() -> Pair {
    let (ta, tb) = support::Fake::pair();
    let world = ta.world.clone();
    let sa = support::TestStorage::default();
    let sb = support::TestStorage::default();
    let a = SecretChatManager::new(ta.clone(), sa.clone(), "11".repeat(32), Options::default())
        .unwrap();
    let b = SecretChatManager::new(tb, sb.clone(), "22".repeat(32), Options::default()).unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(support::peer(2000)).await.unwrap().id;
    support::pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    support::pump(&a, &b, &world).await;
    Pair {
        a,
        b,
        sa,
        sb,
        ta,
        world,
        id,
    }
}
fn received<S: Storage + 'static>(
    m: &SecretChatManager<S, support::Fake>,
) -> Arc<Mutex<Vec<MessageReceived>>> {
    let rows = Arc::new(Mutex::new(Vec::new()));
    let sink = rows.clone();
    m.on("MessageReceived", "rekey_receive", move |e| {
        if let Event::MessageReceived(m) = e {
            sink.lock().unwrap().push(*m);
        }
        Ok(())
    })
    .unwrap();
    rows
}
fn closed<S: Storage + 'static>(
    m: &SecretChatManager<S, support::Fake>,
) -> Arc<Mutex<Vec<(i32, &'static str)>>> {
    let rows = Arc::new(Mutex::new(Vec::new()));
    let sink = rows.clone();
    m.on("ChatClosed", "rekey_closed", move |e| {
        if let Event::ChatClosed {
            chat_id, reason, ..
        } = e
        {
            sink.lock().unwrap().push((chat_id, reason));
        }
        Ok(())
    })
    .unwrap();
    rows
}
fn action_update(
    c: &Chat,
    action: enums::DecryptedMessageAction,
) -> grammers_tl_types::enums::Update {
    let message: enums::DecryptedMessage = types::DecryptedMessageService {
        random_id: 1,
        action,
    }
    .into();
    let wrapper = framing::wrap(
        message,
        144,
        framing::transform_in_seq_no(c.in_seq_no, c.is_outbound).unwrap(),
        framing::transform_out_seq_no(c.out_seq_no, c.is_outbound).unwrap(),
    )
    .unwrap();
    let bytes = crypto::encrypt_frame(
        c.key.as_ref().unwrap(),
        &wrapper.to_bytes(),
        c.out_direction(),
    )
    .unwrap();
    grammers_tl_types::types::UpdateNewEncryptedMessage {
        qts: 1,
        message: grammers_tl_types::types::EncryptedMessageService {
            random_id: 1,
            chat_id: c.id,
            date: 1,
            bytes,
        }
        .into(),
    }
    .into()
}
fn exchange(c: &Chat) -> i64 {
    match c.exchange.as_ref().unwrap() {
        Exchange::Requested { id, .. } | Exchange::Accepted { id, .. } => *id,
    }
}
async fn deliver_b(p: &Pair) {
    loop {
        let next = p
            .world
            .lock()
            .unwrap()
            .updates
            .get_mut(&2000)
            .and_then(std::collections::VecDeque::pop_front);
        if let Some(u) = next {
            p.b.handle_update(&u).await;
        } else {
            break;
        }
    }
}
async fn concurrent_settle(p: &Pair) {
    for _ in 0..1000 {
        let mut batch = Vec::new();
        {
            let mut w = p.world.lock().unwrap();
            for id in [1000, 2000] {
                if let Some(queue) = w.updates.get_mut(&id) {
                    while let Some(update) = queue.pop_front() {
                        batch.push((id, update));
                    }
                }
            }
        }
        if batch.is_empty() {
            return;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for (id, u) in batch {
            let manager = if id == 1000 { p.a.clone() } else { p.b.clone() };
            tasks.spawn(async move {
                manager.handle_update(&u).await;
            });
        }
        while let Some(r) = tasks.join_next().await {
            r.unwrap();
        }
    }
    panic!("concurrent fake update stream did not settle")
}
fn sent_action(c: &Chat) -> Vec<String> {
    c.outbox
        .values()
        .filter_map(|row| {
            let wrapper = framing::unwrap(&row.body).unwrap();
            match wrapper.message {
                enums::DecryptedMessage::Service(s) => {
                    let bytes = s.action.to_bytes();
                    Some(
                        kiaro_secret_chat::name_for_id(u32::from_le_bytes(
                            bytes[..4].try_into().unwrap(),
                        ))
                        .into(),
                    )
                }
                _ => None,
            }
        })
        .collect()
}
pub async fn run(name: &str) {
    if name == "test_a_restart_during_an_exchange_leaves_the_chat_usable" {
        restart().await;
        return;
    }
    let p = pair().await;
    let id = p.id;
    match name {
        "test_an_exchange_completes_and_both_ends_agree_on_the_new_key" => {
            let before = chat(&p.sa, id).key.unwrap();
            p.a.rekey(id).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            let a = chat(&p.sa, id);
            let b = chat(&p.sb, id);
            assert!(a.key.as_ref().unwrap().expose() != before.expose());
            assert!(a.key.as_ref().unwrap().expose() == b.key.as_ref().unwrap().expose());
            assert_eq!(
                a.key.as_ref().unwrap().fingerprint(),
                b.key.as_ref().unwrap().fingerprint()
            );
        }
        "test_the_conversation_continues_after_the_exchange" => {
            let got = received(&p.b);
            p.a.send_message(id, "before", None, None).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            p.a.rekey(id).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            p.a.send_message(id, "after", None, None).await.unwrap();
            p.b.send_message(id, "and back", None, None).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            assert_eq!(
                got.lock()
                    .unwrap()
                    .iter()
                    .map(|x| x.text.as_str())
                    .collect::<Vec<_>>(),
                ["before", "after"]
            );
            assert_eq!(p.a.status(id).unwrap().state, ChatState::Ready);
        }
        "test_a_message_sent_mid_exchange_is_delivered" => {
            let got = received(&p.b);
            p.a.rekey(id).await.unwrap();
            deliver_b(&p).await;
            assert_eq!(p.a.status(id).unwrap().state, ChatState::Rekeying);
            p.a.send_message(id, "sent while rekeying", None, None)
                .await
                .unwrap();
            deliver_b(&p).await;
            assert!(
                got.lock()
                    .unwrap()
                    .iter()
                    .any(|m| m.text == "sent while rekeying")
            );
            support::pump(&p.a, &p.b, &p.world).await;
        }
        "test_a_peers_abort_clears_the_local_exchange" => {
            p.a.rekey(id).await.unwrap();
            let exchange_id = exchange(&chat(&p.sa, id));
            p.a.handle_update(&action_update(
                &chat(&p.sb, id),
                types::DecryptedMessageActionAbortKey { exchange_id }.into(),
            ))
            .await;
            let c = chat(&p.sa, id);
            assert!(c.exchange.is_none());
            assert_eq!(c.state, ChatState::Ready);
        }
        "test_a_bad_fingerprint_in_an_accept_does_not_switch_the_key" => {
            p.a.rekey(id).await.unwrap();
            let c = chat(&p.sa, id);
            let old = c.key.as_ref().unwrap().clone();
            let exchange_id = exchange(&c);
            let g_b = (num_bigint::BigUint::from(1u32) << 2000usize).to_bytes_be();
            p.a.handle_update(&action_update(
                &chat(&p.sb, id),
                types::DecryptedMessageActionAcceptKey {
                    exchange_id,
                    g_b,
                    key_fingerprint: 1,
                }
                .into(),
            ))
            .await;
            let c = chat(&p.sa, id);
            assert!(c.key.as_ref().unwrap().expose() == old.expose());
            assert!(c.exchange.is_none());
            assert_eq!(c.state, ChatState::Ready);
        }
        "test_the_previous_key_is_discarded_once_the_new_one_is_in_use" => {
            p.a.send_message(id, "first", None, None).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            p.a.rekey(id).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            p.b.send_message(id, "under the new key", None, None)
                .await
                .unwrap();
            p.a.send_message(id, "and back", None, None).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            assert!(chat(&p.sa, id).previous_key.is_none());
            assert!(chat(&p.sb, id).previous_key.is_none());
        }
        "test_a_request_key_with_an_unsafe_value_closes_the_chat" => {
            let events = closed(&p.a);
            p.a.handle_update(&action_update(
                &chat(&p.sb, id),
                types::DecryptedMessageActionRequestKey {
                    exchange_id: 5,
                    g_a: {
                        let mut bytes = vec![0; 256];
                        bytes[255] = 1;
                        bytes
                    },
                }
                .into(),
            ))
            .await;
            let c = chat(&p.sa, id);
            assert_eq!(c.state, ChatState::Closed);
            assert!(c.deliveries.is_empty() && c.service_deliveries.is_empty());
            assert_eq!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|x| x.0)
                    .collect::<Vec<_>>(),
                [id]
            );
        }
        "test_flushing_history_does_not_destroy_a_pending_rekey_step" => {
            p.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
            assert!(matches!(
                p.a.rekey(id).await,
                Err(Error::SendPending { .. })
            ));
            p.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
            assert!(matches!(
                p.a.flush_history(id).await,
                Err(Error::SendPending { .. })
            ));
            assert!(
                sent_action(&chat(&p.sa, id))
                    .iter()
                    .any(|x| x == "decryptedMessageActionRequestKey")
            );
        }
        "test_a_peer_flush_leaves_pending_service_actions_alone" => {
            p.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
            assert!(matches!(
                p.a.rekey(id).await,
                Err(Error::SendPending { .. })
            ));
            p.b.flush_history(id).await.unwrap();
            support::pump(&p.a, &p.b, &p.world).await;
            assert_eq!(
                sent_action(&chat(&p.sa, id)),
                ["decryptedMessageActionRequestKey"]
            );
        }
        "test_both_sides_requesting_at_once_end_on_one_key"
        | "test_crossing_requests_settle_on_one_key_with_concurrent_dispatch" => {
            p.a.rekey(id).await.unwrap();
            p.b.rekey(id).await.unwrap();
            if name.contains("concurrent") {
                concurrent_settle(&p).await;
            } else {
                support::pump(&p.a, &p.b, &p.world).await;
            }
            let a = chat(&p.sa, id);
            let b = chat(&p.sb, id);
            assert_eq!((a.state, b.state), (ChatState::Ready, ChatState::Ready));
            assert!(a.key.as_ref().unwrap().expose() == b.key.as_ref().unwrap().expose());
            assert!(a.exchange.is_none() && b.exchange.is_none());
            let got = received(&p.b);
            p.a.send_message(id, "after the collision", None, None)
                .await
                .unwrap();
            if name.contains("concurrent") {
                concurrent_settle(&p).await;
            } else {
                support::pump(&p.a, &p.b, &p.world).await;
            }
            assert_eq!(got.lock().unwrap()[0].text, "after the collision");
        }
        "test_a_rekey_completes_with_concurrent_dispatch" => {
            let old = chat(&p.sa, id).key.unwrap();
            p.a.rekey(id).await.unwrap();
            concurrent_settle(&p).await;
            let a = chat(&p.sa, id);
            let b = chat(&p.sb, id);
            assert!(a.key.as_ref().unwrap().expose() == b.key.as_ref().unwrap().expose());
            assert!(a.key.as_ref().unwrap().expose() != old.expose());
            assert_eq!((a.state, b.state), (ChatState::Ready, ChatState::Ready));
        }
        "test_a_commit_with_the_wrong_fingerprint_closes_the_chat" => {
            p.a.stop().await.unwrap();
            {
                let mut state = p.sa.state.lock().unwrap();
                let c = state.as_mut().unwrap().chats.get_mut(&id).unwrap();
                c.exchange = Some(Exchange::Accepted {
                    id: 5,
                    key: material::key(9, 1),
                });
                c.state = ChatState::Rekeying;
            }
            let revived = SecretChatManager::new(
                p.ta.clone(),
                p.sa.clone(),
                "11".repeat(32),
                Options::default(),
            )
            .unwrap();
            revived.start().await.unwrap();
            let events = closed(&revived);
            let wrong = material::key(9, 1).fingerprint() ^ 1;
            revived
                .handle_update(&action_update(
                    &chat(&p.sb, id),
                    types::DecryptedMessageActionCommitKey {
                        exchange_id: 5,
                        key_fingerprint: wrong,
                    }
                    .into(),
                ))
                .await;
            let c = chat(&p.sa, id);
            assert_eq!(c.state, ChatState::Closed);
            assert_eq!(c.closed_reason, Some(CloseReason::RekeyFingerprint));
            assert_eq!(
                c.closed_reason.unwrap().to_string(),
                "rekey commitment failed its fingerprint check"
            );
            assert_eq!(events.lock().unwrap().len(), 1);
        }
        _ => panic!("unknown rekey manager scenario {name}"),
    }
}
async fn restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("a.json");
    let (ta, tb) = support::Fake::pair();
    let world = ta.world.clone();
    let a = SecretChatManager::new(
        ta.clone(),
        FileStorage::open(&path).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    let b = SecretChatManager::new(
        tb,
        MemoryStorage::default(),
        "22".repeat(32),
        Options::default(),
    )
    .unwrap();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(support::peer(2000)).await.unwrap().id;
    support::pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    support::pump(&a, &b, &world).await;
    a.rekey(id).await.unwrap();
    loop {
        let update = world
            .lock()
            .unwrap()
            .updates
            .get_mut(&2000)
            .and_then(std::collections::VecDeque::pop_front);
        if let Some(update) = update {
            b.handle_update(&update).await;
        } else {
            break;
        }
    }
    let before: State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let before = &before.chats[&id];
    assert_eq!(before.state, ChatState::Rekeying);
    assert!(before.exchange.is_some());
    a.stop().await.unwrap();
    let revived = SecretChatManager::new(
        ta,
        FileStorage::open(&path).unwrap(),
        "11".repeat(32),
        Options::default(),
    )
    .unwrap();
    revived.start().await.unwrap();
    let after: State = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let after = &after.chats[&id];
    assert!(after.key.as_ref().unwrap().expose() == before.key.as_ref().unwrap().expose());
    assert_eq!(exchange(after), exchange(before));
    assert_eq!(after.state, ChatState::Rekeying);
    support::pump(&revived, &b, &world).await;
    assert_eq!(revived.status(id).unwrap().state, ChatState::Ready);
    revived.stop().await.unwrap();
    b.stop().await.unwrap();
}
