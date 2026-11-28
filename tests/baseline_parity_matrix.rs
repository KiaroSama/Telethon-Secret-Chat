use std::sync::Arc;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::manager::{BoxFuture, CoreSecretChatManager, SecretTransport};
use kiaro_secret_chat::protocol::{
    ChatRole, check_dh_config, check_peer_value, describe_protocol_error, dh_public_value,
    dh_shared_key, generate_secret_bytes, key_fingerprint, rekey_collision,
    secret_entities_from_plain, verify_dh_fingerprint,
};
use serde_json::json;

const RFC3526_MODP_2048_HEX: &str = concat!(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
    "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
    "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
    "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
    "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
    "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
    "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
    "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
);

struct LoopbackTransport(&'static str);

impl SecretTransport for LoopbackTransport {
    fn auth_identity(&self) -> &str {
        self.0
    }

    fn send_encrypted<'a>(
        &'a self,
        _chat_id: i64,
        _random_id: i64,
        _frame: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }

    fn discard_encryption<'a>(
        &'a self,
        _chat_id: i64,
        _delete_history: bool,
    ) -> BoxFuture<'a, Result<(), SecretChatError>> {
        Box::pin(async { Ok(()) })
    }
}

fn decimal_str_to_be_bytes(dec: &str) -> Vec<u8> {
    let mut digits = vec![0u8];
    for ch in dec.bytes() {
        let mut carry = (ch - b'0') as u16;
        for byte in digits.iter_mut().rev() {
            let v = (*byte as u16) * 10 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            digits.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    while digits.len() > 1 && digits[0] == 0 {
        digits.remove(0);
    }
    digits
}

#[tokio::test]
async fn full_2048bit_dh_handshake_and_encrypted_exchange_succeeds_and_refuses_composites() {
    let safe_p = hex::decode(RFC3526_MODP_2048_HEX).expect("decode rfc3526 prime");
    assert_eq!(safe_p.len(), 256);

    // RFC 3526 Group 14 with g=2 passes all MTProto 2.0 DH safety checks.
    check_dh_config(2, &safe_p, Some(77)).expect("rfc3526 safe prime accepted");

    // Composite 2048-bit values (both p mod 8 == 1 and p mod 8 == 7 divisible by 3) are refused.
    let mut comp_mod8_1 = vec![0u8; 256];
    comp_mod8_1[0] = 0x80;
    comp_mod8_1[255] = 0x01;
    assert!(check_dh_config(2, &comp_mod8_1, Some(77)).is_err());

    let mut comp_mod8_7 = vec![0u8; 256];
    comp_mod8_7[0] = 0x80;
    comp_mod8_7[255] = 0x07;
    assert!(check_dh_config(2, &comp_mod8_7, Some(77)).is_err());

    // Two peers generate independent OS CSPRNG 2048-bit exponents.
    let mut a_secret = generate_secret_bytes().expect("alice secret");
    let mut b_secret = generate_secret_bytes().expect("bob secret");
    a_secret[0] |= 0x80;
    b_secret[0] |= 0x80;

    let g_bytes = [2u8];
    let ga_dec = dh_public_value(&g_bytes, &a_secret, &safe_p).expect("g^a");
    let gb_dec = dh_public_value(&g_bytes, &b_secret, &safe_p).expect("g^b");
    let ga_bytes = decimal_str_to_be_bytes(&ga_dec);
    let gb_bytes = decimal_str_to_be_bytes(&gb_dec);

    check_peer_value(&ga_bytes, &safe_p, Some(77)).expect("g^a in safe range");
    check_peer_value(&gb_bytes, &safe_p, Some(77)).expect("g^b in safe range");

    let key_alice = dh_shared_key(&gb_bytes, &a_secret, &safe_p, Some(77)).expect("alice key");
    let key_bob = dh_shared_key(&ga_bytes, &b_secret, &safe_p, Some(77)).expect("bob key");
    assert_eq!(key_alice, key_bob);

    let fp = key_fingerprint(&key_alice).expect("fp");
    verify_dh_fingerprint(77, &key_bob, fp).expect("matching fingerprint");
    assert!(verify_dh_fingerprint(77, &key_bob, fp ^ 1).is_err());

    // Rekey collision resolution, entity mapping, and error shaping all agree with baseline.
    assert_eq!(rekey_collision(200, 100).unwrap(), "abandon_theirs");
    assert_eq!(rekey_collision(100, 200).unwrap(), "join_theirs");
    assert_eq!(rekey_collision(150, 150).unwrap(), "abort_both");

    let entities_73 = secret_entities_from_plain(
        json!([
            {"name": "MessageEntityBold", "offset": 0, "length": 4},
            {"name": "MessageEntityCustomEmoji", "offset": 4, "length": 2, "document_id": 999}
        ]),
        73,
    )
    .expect("entities layer 73")
    .expect("non-empty entities at layer 73");
    assert_eq!(entities_73.len(), 1);
    let entities_144 = secret_entities_from_plain(
        json!([
            {"name": "MessageEntityBold", "offset": 0, "length": 4},
            {"name": "MessageEntityCustomEmoji", "offset": 4, "length": 2, "document_id": 999}
        ]),
        144,
    )
    .expect("entities layer 144")
    .expect("non-empty entities at layer 144");
    assert_eq!(entities_144.len(), 2);
    let described =
        describe_protocol_error("UnknownChat", Some(77), "missing").expect("describe error");
    assert!(described.is_some());

    // Exchange an encrypted message using the freshly derived 2048-bit DH key.
    let alice = CoreSecretChatManager::new(Arc::new(LoopbackTransport("auth-dh-alice")));
    let bob = CoreSecretChatManager::new(Arc::new(LoopbackTransport("auth-dh-bob")));
    alice
        .register_established_chat(77, 500, 600, ChatRole::Initiator, &key_alice)
        .await
        .expect("alice register");
    bob.register_established_chat(77, 500, 500, ChatRole::Acceptor, &key_bob)
        .await
        .expect("bob register");

    let wire = alice
        .send_text(77, 4242, "dh-2048-roundtrip", 60)
        .await
        .expect("send over dh key");
    let delivered = bob
        .receive_frame(77, &wire)
        .await
        .expect("receive over dh key");
    assert_eq!(delivered.text.as_deref(), Some("dh-2048-roundtrip"));
}
