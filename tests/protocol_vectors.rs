use std::path::PathBuf;

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::protocol::{
    ChatRole, ChatState, SecretChat, check_dh_config, check_peer_value, compute_msg_key,
    decrypt_frame, derive_keys, encrypt_frame, generate_secret_bytes, key_fingerprint,
    key_visualization, outgoing_layer, pad_payload, raise_remote_layer, require_supported_layer,
    unwrap_frame, wrap_frame,
};
use sha1::{Digest, Sha1};
use sha2::Sha256;

const BURNED_FIXTURE: &str = "tests/vectors/fixtures/official-client-2026-09-29.json";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn independent_kdf(key: &[u8; 256], msg_key: &[u8; 16], x: usize) -> ([u8; 32], [u8; 32]) {
    let mut sha256_a = Sha256::new();
    sha256_a.update(msg_key);
    sha256_a.update(&key[x..x + 36]);
    let a = sha256_a.finalize();

    let mut sha256_b = Sha256::new();
    sha256_b.update(&key[40 + x..40 + x + 36]);
    sha256_b.update(msg_key);
    let b = sha256_b.finalize();

    let mut aes_key = [0u8; 32];
    aes_key[0..8].copy_from_slice(&a[0..8]);
    aes_key[8..24].copy_from_slice(&b[8..24]);
    aes_key[24..32].copy_from_slice(&a[24..32]);

    let mut aes_iv = [0u8; 32];
    aes_iv[0..8].copy_from_slice(&b[0..8]);
    aes_iv[8..24].copy_from_slice(&a[8..24]);
    aes_iv[24..32].copy_from_slice(&b[24..32]);
    (aes_key, aes_iv)
}

#[test]
fn independent_kdf_and_fingerprint_vectors_match() {
    let mut key = [0u8; 256];
    for (i, b) in key.iter_mut().enumerate() {
        *b = ((i * 37 + 11) & 0xff) as u8;
    }
    let payload = vec![0x42u8; 48];

    for x in [0usize, 8usize] {
        let mut h = Sha256::new();
        h.update(&key[88 + x..88 + x + 32]);
        h.update(&payload);
        let expected_msg_key: [u8; 16] = h.finalize()[8..24].try_into().unwrap();

        let got_msg_key = compute_msg_key(&key, &payload, x as i64).expect("compute_msg_key");
        assert_eq!(got_msg_key.as_slice(), &expected_msg_key);

        let (expected_aes_key, expected_aes_iv) = independent_kdf(&key, &expected_msg_key, x);
        let (got_aes_key, got_aes_iv) =
            derive_keys(&key, &expected_msg_key, x as i64).expect("derive_keys");
        assert_eq!(got_aes_key, expected_aes_key);
        assert_eq!(got_aes_iv, expected_aes_iv);
    }

    // Invalid x values outside {0, 8} must be refused.
    assert!(compute_msg_key(&key, &payload, 4).is_err());
    assert!(derive_keys(&key, &[0u8; 16], 16).is_err());

    // Padding bytes must be covered by msg_key.
    let mut p1 = vec![0x11u8; 32];
    p1.extend_from_slice(&[0x00u8; 16]);
    let mut p2 = vec![0x11u8; 32];
    p2.extend_from_slice(&[0x01u8; 16]);
    assert_ne!(
        compute_msg_key(&key, &p1, 0).unwrap(),
        compute_msg_key(&key, &p2, 0).unwrap()
    );

    // Fingerprint is last 64 bits of SHA-1(key) as signed little-endian i64.
    let sha1_digest = Sha1::digest(key);
    let expected_fp = i64::from_le_bytes(sha1_digest[12..20].try_into().unwrap());
    assert_eq!(key_fingerprint(&key).unwrap(), expected_fp);
}

#[test]
fn burned_official_client_fixture_replays_in_order_with_safe_provenance() {
    let fixture_path = repo_root().join(BURNED_FIXTURE);
    let doc: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&fixture_path).expect("read burned official-client fixture"),
    )
    .expect("parse burned fixture json");

    // Provenance rule: burned throwaway chat, no account/chat_id/session fields.
    assert!(doc.get("provenance").is_some());
    assert!(doc.get("chat_id").is_none());
    assert!(doc.get("session").is_none());

    let key = hex::decode(doc["key"].as_str().expect("fixture hex key")).expect("decode key");
    let is_outbound = doc["is_outbound"].as_bool().expect("is_outbound");
    let local_out_seq_no = doc["local_out_seq_no"].as_i64().expect("local_out_seq_no");
    let reply_contains = doc["reply_contains"].as_str().expect("reply_contains");

    let mut chat = SecretChat::new(
        1,
        0,
        0,
        "auth-caller-1",
        if is_outbound {
            ChatRole::Initiator
        } else {
            ChatRole::Acceptor
        },
    );
    chat.adopt_key(&key).expect("adopt burned fixture key");
    chat.set_out_seq_no(local_out_seq_no);
    assert_eq!(chat.state(), ChatState::Ready);

    let frames = doc["frames"].as_array().expect("frames array");
    let mut delivered_texts = Vec::new();
    for entry in frames {
        let frame_bytes = hex::decode(entry["frame"].as_str().unwrap()).unwrap();
        let expected = &entry["expected"];
        let accepted = chat
            .receive_encrypted_frame(&frame_bytes)
            .expect("decrypt and accept burned official-client frame");
        assert_eq!(accepted.layer, expected["layer"].as_i64().unwrap());
        assert_eq!(accepted.in_seq_no, expected["in_seq_no"].as_i64().unwrap());
        assert_eq!(
            accepted.out_seq_no,
            expected["out_seq_no"].as_i64().unwrap()
        );
        assert_eq!(accepted.message_type, expected["type"].as_str().unwrap());
        assert_eq!(accepted.text.as_deref(), expected["text"].as_str());
        if let Some(t) = accepted.text {
            delivered_texts.push(t);
        }
    }

    assert_eq!(chat.in_seq_no(), frames.len() as i64);
    assert!(
        delivered_texts.iter().any(|t| t.contains(reply_contains)),
        "expected reply_contains {reply_contains:?} in {delivered_texts:?}"
    );
}

#[test]
fn framing_layer_clamps_and_refusals_never_advance_state() {
    assert_eq!(outgoing_layer(46).unwrap(), 73);
    assert_eq!(outgoing_layer(73).unwrap(), 73);
    assert_eq!(outgoing_layer(101).unwrap(), 101);
    assert_eq!(outgoing_layer(144).unwrap(), 144);
    assert_eq!(outgoing_layer(200).unwrap(), 144);

    assert_eq!(raise_remote_layer(46, 101).unwrap(), 101);
    assert_eq!(raise_remote_layer(101, 73).unwrap(), 101);

    let err = require_supported_layer(3, 72).unwrap_err();
    assert!(matches!(err, SecretChatError::LayerUnsupported { .. }));
    assert!(require_supported_layer(3, 73).is_ok());

    // Malformed / unparseable wrapper must be refused.
    assert!(matches!(
        unwrap_frame(&[0u8; 8], Some(3)),
        Err(SecretChatError::MessageRejected { .. })
    ));

    // Tampered ciphertext / wrong fingerprint / replay must fail closed without advancing in_seq_no.
    let key = [0x5au8; 256];
    let mut sender = SecretChat::new(7, 1, 2, "auth-a", ChatRole::Initiator);
    sender.adopt_key(&key).unwrap();
    let mut receiver = SecretChat::new(7, 1, 2, "auth-b", ChatRole::Acceptor);
    receiver.adopt_key(&key).unwrap();

    let inner_body = wrap_frame(
        &serde_json::json!({
            "type": "DecryptedMessage",
            "random_id": 1001,
            "ttl": 0,
            "message": "hello-native",
            "media": null,
            "entities": [],
            "via_bot_name": null,
            "reply_to_random_id": null,
            "grouped_id": null
        }),
        73,
        0,
        1,
    )
    .expect("wrap");
    let frame = encrypt_frame(&key, &inner_body, sender.out_x()).expect("encrypt");

    // 1) Tampered fingerprint -> refused, in_seq_no stays 0.
    let mut bad_fp = frame.clone();
    bad_fp[0] ^= 0xff;
    assert!(receiver.receive_encrypted_frame(&bad_fp).is_err());
    assert_eq!(receiver.in_seq_no(), 0);

    // 2) Tampered msg_key -> refused, in_seq_no stays 0.
    let mut bad_mk = frame.clone();
    bad_mk[10] ^= 0x80;
    assert!(receiver.receive_encrypted_frame(&bad_mk).is_err());
    assert_eq!(receiver.in_seq_no(), 0);

    // 3) Tampered ciphertext -> refused, in_seq_no stays 0.
    let mut bad_ct = frame.clone();
    let last = bad_ct.len() - 1;
    bad_ct[last] ^= 0x01;
    assert!(receiver.receive_encrypted_frame(&bad_ct).is_err());
    assert_eq!(receiver.in_seq_no(), 0);

    // 4) Valid frame succeeds once -> in_seq_no becomes 1.
    let ok = receiver
        .receive_encrypted_frame(&frame)
        .expect("valid frame");
    assert_eq!(ok.text.as_deref(), Some("hello-native"));
    assert_eq!(receiver.in_seq_no(), 1);

    // 5) Replay of same frame -> refused, in_seq_no stays 1.
    assert!(receiver.receive_encrypted_frame(&frame).is_err());
    assert_eq!(receiver.in_seq_no(), 1);

    // Padding invariants.
    let padded = pad_payload(&[1, 2, 3, 4]).unwrap();
    assert_eq!(padded.len() % 16, 0);
    assert!(padded.len() >= 4 + 4 + 12);
    assert!(decrypt_frame(&key, &[0u8; 20], 0).is_err());
}

#[test]
fn os_csprng_dh_validation_visualization_and_error_redaction_hold() {
    let s1 = generate_secret_bytes().expect("os csprng secret 1");
    let s2 = generate_secret_bytes().expect("os csprng secret 2");
    assert_eq!(s1.len(), 256);
    assert_eq!(s2.len(), 256);
    assert_ne!(s1, s2);
    assert!(s1.iter().any(|&b| b != 0));

    // Invalid DH config (non-prime / tiny p) and out-of-range peer value must be refused.
    assert!(check_dh_config(2, &[0x00, 0x0f], Some(9)).is_err());
    assert!(check_peer_value(&[0x01], &[0xff; 256], Some(9)).is_err());

    // Key visualization expects 36 bytes (SHA-1[0..16] || SHA-256[0..20]) and returns a 12x12 grid + 64-char hex.
    assert!(key_visualization(&[0xab; 256]).is_err());
    let mut viz_input = Vec::with_capacity(36);
    viz_input.extend_from_slice(&Sha1::digest([0xab; 256])[..16]);
    viz_input.extend_from_slice(&Sha256::digest([0xab; 256])[..20]);
    let viz = key_visualization(&viz_input).expect("visualization");
    let rows = viz["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 12);
    assert_eq!(rows[0].as_array().unwrap().len(), 12);
    assert_eq!(viz["hex"].as_str().unwrap().len(), 64);

    // Principle IV: errors never disclose secret keys, plaintext, ciphertext, or long hex runs.
    let secret_hex = hex::encode([0xdeu8; 32]);
    let plaintext = "top-secret-message-body";
    let errors = [
        SecretChatError::ParameterRejected {
            chat_id: Some(42),
            reason: "the prime is not a safe 2048-bit prime".into(),
        },
        SecretChatError::MessageRejected {
            chat_id: Some(42),
            reason: "msg_key mismatch".into(),
        },
        SecretChatError::ChatNotReady {
            chat_id: 42,
            state: "requested".into(),
        },
        SecretChatError::ChatClosed {
            chat_id: 42,
            reason: "peer discarded the chat".into(),
        },
        SecretChatError::StorageRequired,
        SecretChatError::LayerUnsupported {
            chat_id: 42,
            peer_layer: 8,
        },
        SecretChatError::ResendUnsatisfiable {
            chat_id: 42,
            requested: (5, 9),
            retained_from: 7,
        },
        SecretChatError::StoreCorrupt {
            chat_id: Some(42),
            reason: "key does not match its fingerprint".into(),
        },
        SecretChatError::UnknownChat { chat_id: 42 },
        SecretChatError::ManagerStopping,
        SecretChatError::SendPending {
            chat_id: 42,
            random_id: 99,
            cause: "IoError".into(),
        },
    ];

    for err in &errors {
        let display = err.to_string();
        let debug = format!("{err:?}");
        for rendered in [&display, &debug] {
            assert!(!rendered.to_lowercase().contains(&secret_hex));
            assert!(!rendered.contains(plaintext));
            // No unbroken 32+ hex run in error formatting.
            let mut hex_run = 0usize;
            for ch in rendered.chars() {
                if ch.is_ascii_hexdigit() {
                    hex_run += 1;
                    assert!(hex_run < 32, "unexpected long hex run in {rendered:?}");
                } else {
                    hex_run = 0;
                }
            }
        }
        assert!(display.len() >= 10, "error message too empty: {display:?}");
    }
}
