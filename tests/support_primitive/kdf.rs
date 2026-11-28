#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto::{self, Direction, SharedKey};
use sha2::{Digest, Sha256};
fn raw() -> [u8; 256] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(13))
}
pub fn fixed(offset: u8) {
    super::grammers_kdf::check(offset);
    let v: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/kdf-telethon-reference.json")).unwrap();
    let raw = hex::decode(v["key"].as_str().unwrap()).unwrap();
    let key = SharedKey::from_slice(&raw).unwrap();
    let msg: [u8; 16] = hex::decode(v["msg_key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let (a, b) = crypto::derive_keys(&key, &msg, Direction::try_from(offset).unwrap());
    let reference = &v["directions"][offset.to_string()];
    assert_eq!(hex::encode(a), reference[0]);
    assert_eq!(hex::encode(b), reference[1]);
}
/// Compare fresh random native KDF outputs through Grammers' independent private
/// calc_key via its public MTProto encrypt/decrypt functions, in both directions.
pub fn random(offset: u8) {
    if offset == 0 {
        super::frame_properties::check();
    }
    let d = Direction::try_from(offset).unwrap();
    for _ in 0..100 {
        let raw = crypto::random_bytes::<256>().unwrap();
        let key = SharedKey::from_slice(&raw).unwrap();
        let plaintext = crypto::random_bytes::<64>().unwrap();
        let auth = grammers_crypto::AuthKey::from_bytes(raw);
        if offset == 0 {
            let mut reference = plaintext;
            let msg = grammers_crypto::encrypt_data_v2(&mut reference, &auth);
            let (aes, iv) = crypto::derive_keys(&key, &msg, d);
            let mut native = plaintext;
            grammers_crypto::aes::ige_encrypt(&mut native, &aes, &iv);
            assert_eq!(native, reference);
        } else {
            let msg = crypto::compute_msg_key(&key, &plaintext, d);
            let (aes, iv) = crypto::derive_keys(&key, &msg, d);
            let mut encrypted = plaintext;
            grammers_crypto::aes::ige_encrypt(&mut encrypted, &aes, &iv);
            let mut frame = Vec::new();
            frame.extend_from_slice(&auth.key_id());
            frame.extend_from_slice(&msg);
            frame.extend_from_slice(&encrypted);
            assert_eq!(
                grammers_crypto::decrypt_data_v2(&mut frame, &auth).unwrap(),
                &plaintext
            );
        }
    }
}
pub fn other(name: &str) {
    let key = SharedKey::from_slice(&raw()).unwrap();
    match name {
        "test_the_two_directions_do_not_share_a_key" => {
            let msg = crypto::random_bytes().unwrap();
            assert_ne!(
                crypto::derive_keys(&key, &msg, Direction::Initiator),
                crypto::derive_keys(&key, &msg, Direction::Responder)
            );
        }
        "test_an_x_outside_the_protocol_is_refused" => {
            assert!(Direction::try_from(4).is_err());
            for offset in [1, 7, 9, 255] {
                assert!(Direction::try_from(offset).is_err());
            }
            assert_eq!(Direction::try_from(0).unwrap(), Direction::Initiator);
            assert_eq!(Direction::try_from(8).unwrap(), Direction::Responder);
        }
        "test_the_key_and_iv_are_the_sizes_aes_256_ige_needs" => {
            let (a, b) =
                crypto::derive_keys(&key, &crypto::random_bytes().unwrap(), Direction::Initiator);
            assert_eq!((a.len(), b.len()), (32, 32));
        }
        "test_msg_key_is_the_middle_128_bits_of_the_documented_hash" => {
            for d in [Direction::Initiator, Direction::Responder] {
                let payload = crypto::random_bytes::<64>().unwrap();
                let x = d.offset();
                let mut all = raw()[88 + x..120 + x].to_vec();
                all.extend_from_slice(&payload);
                assert_eq!(
                    crypto::compute_msg_key(&key, &payload, d),
                    Sha256::digest(all)[8..24]
                );
            }
        }
        "test_msg_key_covers_the_padding_too" => {
            let mut a = crypto::random_bytes::<32>().unwrap().to_vec();
            let mut b = a.clone();
            a.extend_from_slice(&[0; 16]);
            b.extend_from_slice(&[1; 16]);
            assert_ne!(
                crypto::compute_msg_key(&key, &a, Direction::Initiator),
                crypto::compute_msg_key(&key, &b, Direction::Initiator)
            );
        }
        "test_the_fingerprint_is_the_last_64_bits_of_sha1_little_endian" => {
            let digest = sha1::Sha1::digest(raw());
            assert_eq!(
                key.fingerprint(),
                i64::from_le_bytes(digest[12..20].try_into().unwrap())
            );
        }
        _ => panic!("unknown KDF scenario {name}"),
    }
}
