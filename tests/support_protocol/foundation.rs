#![allow(dead_code, clippy::unwrap_used)] // Three targets reuse distinct regression families.
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord, Envelope},
    crypto::SharedKey,
    enums,
    protocol::Protocol,
    sequence,
    storage::MemoryStorage,
    types,
};
use std::sync::OnceLock;
fn dh() -> DhRecord {
    static DH: OnceLock<DhRecord> = OnceLock::new();
    DH.get_or_init(|| DhRecord {
        g: 2,
        prime: hex::decode(concat!(
            "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
            "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
            "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
            "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
            "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
            "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
            "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
            "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF"
        ))
        .unwrap(),
    })
    .clone()
}
fn protocol(outbound: bool) -> Protocol<MemoryStorage> {
    let mut protocol = Protocol::open(MemoryStorage::default(), "11".repeat(32)).unwrap();
    protocol.start().unwrap();
    protocol
        .establish(
            Chat::new(1, 2, 3, outbound, ChatState::Waiting),
            SharedKey::from_slice(&[7; 256]).unwrap(),
            dh(),
            1,
        )
        .unwrap();
    protocol
}
fn envelope(raw: i32, peer_in: i32, layer: i32, outbound: bool) -> Envelope {
    let m = enums::DecryptedMessage::from(types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: i64::from(raw + 1),
        ttl: 0,
        message: "hi".into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    });
    Envelope {
        body: kiaro_secret_chat::framing::wrap(
            m,
            layer,
            2 * peer_in + i32::from(outbound),
            2 * raw + i32::from(!outbound),
        )
        .unwrap()
        .to_bytes(),
        file: None,
        date: 1,
    }
}
pub fn pending_send_retains_exact_identity_and_ciphertext() {
    let mut p = protocol(true);
    let effect = p.send_message(1, "hello".into(), None, None).unwrap();
    let pending = p.retry_pending(1).unwrap();
    let row = pending
        .iter()
        .find(|e| e.outgoing.random_id == effect.outgoing.random_id)
        .unwrap();
    assert_eq!(row.outgoing.frame, effect.outgoing.frame);
    assert_eq!(row.outgoing.seq_no, effect.outgoing.seq_no);
}
pub fn confirmed_send_remains_available_for_peer_resend() {
    let mut p = protocol(true);
    let effect = p.send_message(1, "hello".into(), None, None).unwrap();
    p.confirm_sent(&effect).unwrap();
    assert!(
        p.retry_pending(1)
            .unwrap()
            .iter()
            .all(|e| e.outgoing.random_id != effect.outgoing.random_id)
    );
    let mut p = protocol(true);
    let original = p
        .send_message(1, "withdraw before the old RPC returns".into(), None, None)
        .unwrap();
    let request = peer_frame(
        &service_envelope(0, 0, enums::DecryptedMessageAction::FlushHistory),
        7,
    );
    p.receive(1, &request, None, 1).unwrap();
    p.apply_service_delivery(1, 0).unwrap();
    let rewritten = p
        .retry_pending(1)
        .unwrap()
        .into_iter()
        .find(|e| e.outgoing.random_id == original.outgoing.random_id)
        .unwrap();
    assert!(rewritten.outgoing.service);
    assert_ne!(rewritten.outgoing.frame, original.outgoing.frame);
    p.confirm_sent(&original).unwrap();
    assert!(
        p.chat_record(1).unwrap().outbox[&original.outgoing.seq_no].pending,
        "old plaintext RPC success must not confirm unsent withdrawal ciphertext"
    );
    p.confirm_sent(&rewritten).unwrap();
    assert!(!p.chat_record(1).unwrap().outbox[&original.outgoing.seq_no].pending);
    p.mutate_chat(1, |chat| {
        chat.outbox.remove(&original.outgoing.seq_no);
        Ok(())
    })
    .unwrap();
    p.confirm_sent(&original).unwrap();
}
pub fn two_sides_exchange_and_acknowledge_delivery() {
    let mut a = protocol(true);
    let mut b = protocol(false);
    let sent = a.send_message(1, "hello".into(), None, None).unwrap();
    let effects = b.receive(1, &sent.outgoing.frame, None, 1).unwrap();
    assert!(
        effects.events.is_empty(),
        "a queued gap must not publish delivery before its predecessor"
    );
    assert_eq!(b.pending_deliveries(1).unwrap().len(), 0); // notify-layer sequence 0 must arrive first
    let notify = a.retry_pending(1).unwrap().remove(0);
    b.receive(1, &notify.outgoing.frame, None, 1).unwrap();
    assert_eq!(b.pending_deliveries(1).unwrap(), vec![1]);
    let stored = b.delivery(1, 1).unwrap().unwrap();
    assert!(kiaro_secret_chat::framing::unwrap(&stored.body).is_ok());
    b.acknowledge_delivery(1, 1).unwrap();
    assert!(b.pending_deliveries(1).unwrap().is_empty());
}
pub fn replay_is_not_redelivered() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    let env = envelope(0, 0, 144, true);
    assert_eq!(
        sequence::accept(&mut chat, env.clone())
            .unwrap()
            .ready
            .len(),
        1
    );
    assert!(sequence::accept(&mut chat, env).unwrap().ready.is_empty());
    assert_eq!(chat.in_seq_no, 1);
}
pub fn gap_requests_once_and_closes_in_order() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    let one = sequence::accept(&mut chat, envelope(1, 0, 144, true)).unwrap();
    assert_eq!(one.resend, Some((0, 0)));
    assert!(
        sequence::accept(&mut chat, envelope(2, 0, 144, true))
            .unwrap()
            .resend
            .is_none()
    );
    let result = sequence::accept(&mut chat, envelope(0, 0, 144, true)).unwrap();
    assert_eq!(
        result.ready.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(chat.gaps.is_empty());
    assert_eq!(chat.gap_end, None);
}
pub fn sequence_mirroring_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(0, 0, 144, false)).is_err());
}
pub fn excessive_gap_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(1001, 0, 144, true)).is_err());
    assert!(chat.gaps.is_empty());
}
pub fn unsent_peer_echo_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(0, 1, 144, true)).is_err());
}
pub fn backwards_wrapper_layer_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    sequence::accept(&mut chat, envelope(0, 0, 144, true)).unwrap();
    assert!(sequence::accept(&mut chat, envelope(1, 0, 73, true)).is_err());
}
pub fn paused_chat_refuses_sends_until_explicit_resume() {
    let mut p = protocol(true);
    p.pause(1).unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_err());
    p.resume(1).unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_ok());
}
pub fn stopping_refuses_further_effects() {
    let mut p = protocol(true);
    p.stop().unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_err());
}
pub fn closed_tombstone_prevents_revival() {
    let mut p = protocol(true);
    p.close(1).unwrap();
    assert!(
        p.establish(
            Chat::new(1, 2, 3, true, ChatState::Waiting),
            SharedKey::from_slice(&[7; 256]).unwrap(),
            dh(),
            1
        )
        .is_err()
    );
    assert!(p.send_message(1, "hi".into(), None, None).is_err());
}
pub fn key_visualization_survives_rekey() {
    let mut p = protocol(true);
    let first = p.snapshot(1).unwrap().key_hash;
    p.rekey(1).unwrap();
    assert_eq!(p.snapshot(1).unwrap().key_hash, first);
}

fn service_envelope(raw: i32, peer_in: i32, action: enums::DecryptedMessageAction) -> Vec<u8> {
    kiaro_secret_chat::framing::wrap(
        types::DecryptedMessageService {
            random_id: 1000 + i64::from(raw),
            action,
        }
        .into(),
        144,
        2 * peer_in + 1,
        2 * raw,
    )
    .unwrap()
    .to_bytes()
}
fn peer_frame(body: &[u8], byte: u8) -> Vec<u8> {
    kiaro_secret_chat::crypto::encrypt_frame(
        &SharedKey::from_slice(&[byte; 256]).unwrap(),
        body,
        kiaro_secret_chat::crypto::Direction::Responder,
    )
    .unwrap()
}
pub fn test_a_resend_request_is_answered_even_while_it_is_out_of_order() {
    let mut p = protocol(true);
    let original = p.send_message(1, "retained".into(), None, None).unwrap();
    p.confirm_sent(&original).unwrap();
    let request = peer_frame(
        &service_envelope(
            2,
            0,
            types::DecryptedMessageActionResend {
                start_seq_no: 3,
                end_seq_no: 3,
            }
            .into(),
        ),
        7,
    );
    let effects = p.receive(1, &request, None, 1).unwrap();
    assert!(
        effects
            .sends
            .iter()
            .any(|effect| effect.outgoing.frame == original.outgoing.frame)
    );
    assert!(p.receive(1, &request, None, 1).unwrap().sends.is_empty());
    for raw in [0, 1] {
        let body = envelope(raw, 0, 144, true).body;
        let effects = p.receive(1, &peer_frame(&body, 7), None, 1).unwrap();
        assert!(
            effects
                .sends
                .iter()
                .all(|effect| effect.outgoing.frame != original.outgoing.frame)
        );
    }
}
pub fn test_a_resend_answer_lost_on_the_network_is_sent_again() {
    let mut p = protocol(true);
    let original = p.send_message(1, "retained".into(), None, None).unwrap();
    p.confirm_sent(&original).unwrap();
    let request = peer_frame(
        &service_envelope(
            0,
            0,
            types::DecryptedMessageActionResend {
                start_seq_no: 3,
                end_seq_no: 3,
            }
            .into(),
        ),
        7,
    );
    p.receive(1, &request, None, 1).unwrap();
    p.stop().unwrap();
    let recovered = p.start().unwrap();
    assert!(
        recovered
            .iter()
            .any(|effect| effect.outgoing.frame == original.outgoing.frame)
    );
    assert!(
        p.retry_pending(1)
            .unwrap()
            .iter()
            .any(|effect| effect.outgoing.random_id == original.outgoing.random_id)
    );
}
pub fn resend_precedes_later_ready_acknowledgement_pruning() {
    let mut p = protocol(true);
    let original = p.send_message(1, "retained".into(), None, None).unwrap();
    p.confirm_sent(&original).unwrap();
    let later = envelope(1, 2, 144, true);
    p.receive(1, &peer_frame(&later.body, 7), None, 1).unwrap();
    let earlier = service_envelope(
        0,
        1,
        types::DecryptedMessageActionResend {
            start_seq_no: 3,
            end_seq_no: 3,
        }
        .into(),
    );
    let effects = p.receive(1, &peer_frame(&earlier, 7), None, 1).unwrap();
    assert!(
        effects
            .sends
            .iter()
            .any(|effect| effect.outgoing.frame == original.outgoing.frame)
    );
    assert!(
        p.retry_pending(1)
            .unwrap()
            .iter()
            .any(|effect| effect.outgoing.seq_no == 3)
    );
}
pub fn fallback_adoption_queues_new_key_confirmation_with_an_open_gap() {
    let mut p = protocol(true);
    p.mutate_chat(1, |chat| {
        chat.state = ChatState::Rekeying;
        chat.exchange = Some(kiaro_secret_chat::chat::Exchange::Accepted {
            id: 123,
            key: SharedKey::from_slice(&[8; 256]).unwrap(),
        });
        Ok(())
    })
    .unwrap();
    let body = envelope(1, 0, 144, true).body;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let effects = p.receive(1, &peer_frame(&body, 8), None, 1).unwrap();
    let new_key = SharedKey::from_slice(&[8; 256]).unwrap();
    assert!(effects.sends.iter().any(|effect|{
        if effect.outgoing.frame[..8]!=new_key.fingerprint().to_le_bytes(){return false;}
        let body=kiaro_secret_chat::crypto::decrypt_frame(&new_key,&effect.outgoing.frame,kiaro_secret_chat::crypto::Direction::Initiator).unwrap();
        matches!(kiaro_secret_chat::framing::unwrap(&body).unwrap().message,enums::DecryptedMessage::Service(service) if matches!(service.action,enums::DecryptedMessageAction::Noop))
    }));
    let chat = p.chat_record(1).unwrap();
    assert!(chat.previous_key.is_some());
    assert!(chat.exchange.is_none());
    assert!(
        chat.rekeyed_at >= before,
        "a newly adopted key must use the local adoption clock"
    );
    assert_eq!(chat.gaps.values().next().unwrap().date, 1);
}
