use super::{fake, fixture::*, soon, tl};
use crate::{
    Serializable,
    chat::{Chat, ChatState, DhRecord},
    crypto::{Direction, encrypt_frame},
    dh::{DhConfig, SecretExponent},
    types::DecryptedMessage,
};

#[tokio::test]
async fn startup_replay_retires_terminal_rows_and_recovers_healthy_chat() {
    let f = started().await;
    let mut closed = chat(7);
    closed.close();
    closed.paused = true;
    f.manager.test_install_chat(closed).unwrap();
    install(&f, 9);
    let absent = tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessageService {
            random_id: 82,
            chat_id: 8,
            date: 124,
            bytes: vec![],
        }
        .into(),
        qts: 42,
    };
    capture(
        &f,
        vec![
            update(7, 41, 81, vec![]),
            absent.into(),
            update(9, 43, 83, frame(0, 83, "healthy after terminal rows")),
        ],
        2,
    )
    .await;
    let batches = f.ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].updates().unwrap().0.len(), 3);
    assert!(batches[0].applied.is_empty());
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding.clone())).await;
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    let held = recovered.pending_deliveries(9).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(message(&held[0]).message.random_id(), 83);
    assert_eq!(ingress.core.lock().unwrap().state.chats[&9].in_seq_no, 1);
    recovered.acknowledge_delivery(9, held[0].seq_no).unwrap();
    soon(recovered.stop()).await.unwrap();

    let (recovered, ingress, _pool) = soon(reopen(&path, binding)).await;
    assert!(ingress.pending_batches().unwrap().is_empty());
    assert!(recovered.pending_deliveries(9).unwrap().is_empty());
    {
        let core = ingress.core.lock().unwrap();
        assert_eq!(core.state.chats[&7].state, ChatState::Closed);
        assert_eq!(core.state.chats[&7].in_seq_no, 0);
        assert!(!core.state.chats.contains_key(&8));
        assert_eq!(core.state.chats[&9].in_seq_no, 1);
    }
    soon(recovered.stop()).await.unwrap();
}

#[tokio::test]
async fn startup_replay_defers_nonready_rows_until_chat_is_ready() {
    let f = started().await;
    let prime = fake::prime();
    let dh = DhConfig::new(2, &prime).unwrap();
    let local = SecretExponent::from_bytes([0x81; 256]);
    let remote = SecretExponent::from_bytes([0x82; 256]);
    let peer_public = dh.public_value(&remote);
    let key = dh.shared_key(&peer_public, &local).unwrap();
    let mut waiting = Chat::new(7, 88, 2000, false, ChatState::Waiting);
    waiting.handshake_secret = Some(local);
    waiting.peer_public = Some(peer_public.clone());
    waiting.dh = Some(DhRecord { g: 2, prime });
    f.manager.test_install_chat(waiting).unwrap();
    install(&f, 8);

    let body = DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 91,
        ttl: 0,
        message: "held during establishment".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    };
    let wrapper = crate::framing::wrap(body.into(), 144, 0, 1).unwrap();
    let wire = encrypt_frame(&key, &wrapper.to_bytes(), Direction::Initiator).unwrap();
    capture(
        &f,
        vec![
            update(7, 41, 91, wire),
            update(8, 42, 92, frame(0, 92, "healthy during establishment")),
        ],
        2,
    )
    .await;
    let binding = f.ingress.binding().unwrap();
    let path = f.dir.path().join("state/custody.bin");
    soon(f.manager.stop()).await.unwrap();

    let (recovered, ingress, pool) = soon(reopen(&path, binding)).await;
    let batches = ingress.pending_batches().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].cursor, 0);
    assert_eq!(batches[0].applied, [1].into());
    assert_eq!(recovered.status(7).unwrap().state, ChatState::Waiting);
    assert!(recovered.pending_deliveries(7).unwrap().is_empty());
    let held = recovered.pending_deliveries(8).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(message(&held[0]).message.random_id(), 92);

    let established = tl::types::UpdateEncryption {
        chat: tl::types::EncryptedChat {
            id: 7,
            access_hash: 88,
            date: 124,
            admin_id: 2000,
            participant_id: 1000,
            g_a_or_b: peer_public,
            key_fingerprint: key.fingerprint(),
        }
        .into(),
        date: 124,
    }
    .into();
    let (tx, mut receiver) = super::super::test_fixture::receiver(&ingress, &pool).await;
    let (restored, _, _) = soon(receiver.next()).await.unwrap();
    assert_eq!(restored.len(), 1);
    assert!(matches!(
        restored[0].0,
        tl::enums::Update::NewEncryptedMessage(_)
    ));
    super::super::test_fixture::capture(&mut receiver, &tx, envelope(vec![established], 3)).await;
    soon(recovered.drain_ingress()).await.unwrap();
    assert_eq!(recovered.status(7).unwrap().state, ChatState::Ready);
    soon(recovered.drain_ingress()).await.unwrap();
    let held = recovered.pending_deliveries(7).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(message(&held[0]).message.random_id(), 91);
    assert!(ingress.pending_batches().unwrap().is_empty());
    {
        let core = ingress.core.lock().unwrap();
        assert_eq!(core.state.chats[&7].in_seq_no, 1);
        assert_eq!(core.state.chats[&8].in_seq_no, 1);
    }
    soon(recovered.stop()).await.unwrap();
}
