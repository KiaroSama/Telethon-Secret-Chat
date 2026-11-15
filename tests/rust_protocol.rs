#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord, Envelope},
    crypto::SharedKey,
    dh::{DhConfig, SecretExponent},
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
#[test]
fn shared_key_dh_agreement() {
    let record = dh();
    let dh = DhConfig::new(record.g, &record.prime).unwrap();
    let a = SecretExponent::generate().unwrap();
    let b = SecretExponent::generate().unwrap();
    let key_a = dh.shared_key(&dh.public_value(&b), &a).unwrap();
    let key_b = dh.shared_key(&dh.public_value(&a), &b).unwrap();
    assert_eq!(key_a.expose(), key_b.expose());
}
#[test]
fn dh_rejects_small_subgroup_values() {
    let record = dh();
    let dh = DhConfig::new(record.g, &record.prime).unwrap();
    for bytes in [vec![], vec![0], vec![1], vec![255; 257]] {
        assert!(dh.check_peer(&bytes).is_err());
    }
}
#[test]
fn dh_refuses_composite_passing_residue() {
    let mut prime = vec![0; 256];
    prime[0] = 128;
    prime[255] = 7;
    assert!(DhConfig::new(2, &prime).is_err());
}
#[test]
fn dh_refuses_wrong_generators() {
    for g in [0, 1, 8, 9, u32::MAX] {
        assert!(DhConfig::new(g, &dh().prime).is_err());
    }
}
#[test]
fn dh_refuses_wrong_width() {
    for length in [0, 1, 128, 255, 257] {
        assert!(DhConfig::new(2, &vec![255; length]).is_err());
    }
}
#[test]
fn pending_send_retains_exact_identity_and_ciphertext() {
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
#[test]
fn confirmed_send_remains_available_for_peer_resend() {
    let mut p = protocol(true);
    let effect = p.send_message(1, "hello".into(), None, None).unwrap();
    p.confirm_sent(1, effect.outgoing.seq_no, effect.outgoing.random_id)
        .unwrap();
    assert!(
        p.retry_pending(1)
            .unwrap()
            .iter()
            .all(|e| e.outgoing.random_id != effect.outgoing.random_id)
    );
}
#[test]
fn two_sides_exchange_and_acknowledge_delivery() {
    let mut a = protocol(true);
    let mut b = protocol(false);
    let sent = a.send_message(1, "hello".into(), None, None).unwrap();
    let effects = b.receive(1, &sent.outgoing.frame, None, 1).unwrap();
    assert!(
        effects
            .events
            .iter()
            .all(|e| !matches!(e, kiaro_secret_chat::protocol::Event::DecryptFailed { .. }))
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
#[test]
fn replay_is_not_redelivered() {
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
#[test]
fn gap_requests_once_and_closes_in_order() {
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
#[test]
fn sequence_mirroring_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(0, 0, 144, false)).is_err());
}
#[test]
fn excessive_gap_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(1001, 0, 144, true)).is_err());
    assert!(chat.gaps.is_empty());
}
#[test]
fn unsent_peer_echo_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    assert!(sequence::accept(&mut chat, envelope(0, 1, 144, true)).is_err());
}
#[test]
fn backwards_wrapper_layer_is_refused() {
    let mut chat = Chat::new(1, 2, 3, true, ChatState::Ready);
    sequence::accept(&mut chat, envelope(0, 0, 144, true)).unwrap();
    assert!(sequence::accept(&mut chat, envelope(1, 0, 73, true)).is_err());
}
#[test]
fn paused_chat_refuses_sends_until_explicit_resume() {
    let mut p = protocol(true);
    p.pause(1).unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_err());
    p.resume(1).unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_ok());
}
#[test]
fn stopping_refuses_further_effects() {
    let mut p = protocol(true);
    p.stop().unwrap();
    assert!(p.send_message(1, "hi".into(), None, None).is_err());
}
#[test]
fn closed_tombstone_prevents_revival() {
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
#[test]
fn key_visualization_survives_rekey() {
    let mut p = protocol(true);
    let first = p.snapshot(1).unwrap().key_hash;
    p.rekey(1).unwrap();
    assert_eq!(p.snapshot(1).unwrap().key_hash, first);
}
