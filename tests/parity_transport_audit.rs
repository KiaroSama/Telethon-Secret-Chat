#![forbid(unsafe_code)]

use kiaro_secret_chat::errors::SecretChatError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use kiaro_secret_chat::establishment::{
    EstablishedChat, SecretChatEstablisher, render_key_visualization_png,
};
use kiaro_secret_chat::grammers_client::Client;
use kiaro_secret_chat::grammers_mtsender::SenderPool;
use kiaro_secret_chat::grammers_session::{Session, storages::MemorySession};
use kiaro_secret_chat::manager::{BoxFuture, SecretTransport};
use kiaro_secret_chat::protocol::{
    ChatRole, MAX_ENCRYPTED_FRAME_LEN, SecretChat, dh_public_value, dh_shared_key, key_fingerprint,
};
use kiaro_secret_chat::transport::{
    DhConfigData, EncryptedChatWire, EncryptedFileRef, EncryptedFileUpload, EstablishmentTransport,
};
use num_bigint::BigUint;

fn established(role: ChatRole) -> EstablishedChat {
    let key = [0x42; 256];
    let mut chat = SecretChat::new(-7, 970, 20, "synthetic-transport-audit", role);
    chat.adopt_key(&key).unwrap();
    let (key_picture, _) = render_key_visualization_png(&key, &[], 1).unwrap();
    EstablishedChat {
        chat,
        g_a: Vec::new(),
        key_picture,
    }
}

#[test]
fn direct_text_rejects_a_frame_over_the_receive_limit_without_consuming_sequence() {
    let mut alice = established(ChatRole::Initiator);
    let text = "x".repeat(MAX_ENCRYPTED_FRAME_LEN);
    assert!(
        matches!(
            alice.encrypt_text(1, &text, 0),
            Err(SecretChatError::ParameterRejected { .. })
        ),
        "the serialized wrapper, padding and encrypted header must fit the receive limit"
    );
    assert_eq!(alice.chat.out_seq_no(), 0);
    let frame = alice.encrypt_text(2, "still usable", 0).unwrap();
    let mut bob = established(ChatRole::Acceptor);
    assert_eq!(
        bob.chat
            .receive_encrypted_frame(&frame)
            .unwrap()
            .text
            .as_deref(),
        Some("still usable")
    );
}

#[test]
fn direct_text_near_the_receive_limit_remains_decryptable() {
    let mut alice = established(ChatRole::Initiator);
    let text = "x".repeat(MAX_ENCRYPTED_FRAME_LEN - 128);
    let frame = alice.encrypt_text(1, &text, 0).unwrap();
    assert!(frame.len() <= MAX_ENCRYPTED_FRAME_LEN);
    let mut bob = established(ChatRole::Acceptor);
    assert_eq!(
        bob.chat.receive_encrypted_frame(&frame).unwrap().text,
        Some(text)
    );
    assert_eq!(alice.chat.out_seq_no(), 1);
    assert_eq!(bob.chat.in_seq_no(), 1);
}

type Rpc<'a, T> = BoxFuture<'a, Result<T, SecretChatError>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentityChange {
    Never,
    Dh,
    Request,
    Accept,
}

#[derive(Clone)]
struct HandshakeTransport {
    changed: Arc<AtomicBool>,
    change_at: IdentityChange,
    side_effects: Arc<AtomicUsize>,
    g_a: Vec<u8>,
}

impl HandshakeTransport {
    fn new(change_at: IdentityChange) -> Self {
        let g_a = BigUint::parse_bytes(
            dh_public_value(&[2], &[0x12; 256], &dh_config().p)
                .unwrap()
                .as_bytes(),
            10,
        )
        .unwrap()
        .to_bytes_be();
        Self {
            changed: Arc::new(AtomicBool::new(false)),
            change_at,
            side_effects: Arc::default(),
            g_a,
        }
    }

    fn change_identity(&self, stage: IdentityChange) {
        if self.change_at == stage {
            self.changed.store(true, Ordering::SeqCst);
        }
    }

    fn requested(&self) -> EncryptedChatWire {
        EncryptedChatWire::Requested {
            chat_id: -7,
            access_hash: 970,
            admin_id: 10,
            participant_id: 20,
            g_a: self.g_a.clone(),
            date: 1,
        }
    }
}

fn dh_config() -> DhConfigData {
    // RFC 3526 group 14: public test parameters, with no live credentials.
    let p = hex::decode(concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
        "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
        "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
        "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    ))
    .unwrap();
    DhConfigData {
        g: 2,
        p,
        version: 1,
        random: vec![0x6d; 256],
    }
}

impl SecretTransport for HandshakeTransport {
    fn auth_identity(&self) -> &str {
        if self.changed.load(Ordering::SeqCst) {
            "changed-authorization"
        } else {
            "original-authorization"
        }
    }

    fn send_encrypted(&self, _: i64, _: i64, _: Vec<u8>) -> Rpc<'_, ()> {
        Box::pin(async { panic!("handshake tests never send message RPCs") })
    }

    fn discard_encryption(&self, _: i64, _: bool) -> Rpc<'_, ()> {
        Box::pin(async { panic!("handshake tests never discard chats") })
    }
}

impl EstablishmentTransport for HandshakeTransport {
    fn get_dh_config(&self, version: i32, random_length: i32) -> Rpc<'_, DhConfigData> {
        Box::pin(async move {
            assert_eq!((version, random_length), (0, 256));
            self.change_identity(IdentityChange::Dh);
            Ok(dh_config())
        })
    }

    fn request_encryption(
        &self,
        peer: i64,
        hash: i64,
        _: i32,
        _: Vec<u8>,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            assert_eq!((peer, hash), (20, 25));
            self.side_effects.fetch_add(1, Ordering::SeqCst);
            self.change_identity(IdentityChange::Request);
            Ok(EncryptedChatWire::Waiting {
                chat_id: -7,
                access_hash: 970,
                admin_id: 10,
                participant_id: 20,
                date: 1,
            })
        })
    }

    fn accept_encryption(
        &self,
        chat_id: i32,
        hash: i64,
        g_b: Vec<u8>,
        fingerprint: i64,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            assert_eq!((chat_id, hash), (-7, 970));
            let peer_key = dh_shared_key(&g_b, &[0x12; 256], &dh_config().p, Some(-7))?;
            assert_eq!(key_fingerprint(&peer_key)?, fingerprint);
            self.side_effects.fetch_add(1, Ordering::SeqCst);
            self.change_identity(IdentityChange::Accept);
            Ok(EncryptedChatWire::Active {
                chat_id,
                access_hash: hash,
                admin_id: 10,
                participant_id: 20,
                g_a_or_b: self.g_a.clone(),
                key_fingerprint: fingerprint,
                date: 2,
            })
        })
    }

    fn discard_encryption(&self, _: i32, _: bool) -> Rpc<'_, bool> {
        Box::pin(async { panic!("handshake tests never discard chats") })
    }

    fn send_encrypted_file(
        &self,
        _: i32,
        _: i64,
        _: i64,
        _: Vec<u8>,
        _: EncryptedFileUpload,
    ) -> Rpc<'_, i32> {
        Box::pin(async { panic!("handshake tests never send files") })
    }

    fn upload_encrypted_bytes(&self, _: Vec<u8>, _: i32) -> Rpc<'_, EncryptedFileUpload> {
        Box::pin(async { panic!("handshake tests never upload files") })
    }

    fn download_encrypted_bytes(&self, _: &EncryptedFileRef) -> Rpc<'_, Vec<u8>> {
        Box::pin(async { panic!("handshake tests never download files") })
    }
}

#[tokio::test]
async fn request_stops_before_remote_mutation_when_authorization_changes_during_dh() {
    let transport = HandshakeTransport::new(IdentityChange::Dh);
    let establisher = SecretChatEstablisher::new(transport.clone());
    assert!(establisher.request_chat(20, 25).await.is_err());
    assert_eq!(transport.side_effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn accept_stops_before_remote_mutation_when_authorization_changes_during_dh() {
    let transport = HandshakeTransport::new(IdentityChange::Dh);
    let establisher = SecretChatEstablisher::new(transport.clone());
    assert!(
        establisher
            .accept_chat(&transport.requested())
            .await
            .is_err()
    );
    assert_eq!(transport.side_effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn request_refuses_to_publish_a_pending_chat_after_authorization_changes() {
    let establisher = SecretChatEstablisher::new(HandshakeTransport::new(IdentityChange::Request));
    assert!(establisher.request_chat(20, 25).await.is_err());
}

#[tokio::test]
async fn accept_refuses_to_publish_a_ready_chat_after_authorization_changes() {
    let transport = HandshakeTransport::new(IdentityChange::Accept);
    let establisher = SecretChatEstablisher::new(transport.clone());
    assert!(
        establisher
            .accept_chat(&transport.requested())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unchanged_authorization_can_request_and_accept() {
    let transport = HandshakeTransport::new(IdentityChange::Never);
    let establisher = SecretChatEstablisher::new(transport.clone());
    assert_eq!(
        establisher.request_chat(20, 25).await.unwrap().chat_id(),
        -7
    );
    let accepted = establisher
        .accept_chat(&transport.requested())
        .await
        .unwrap();
    assert_eq!(accepted.chat.caller_auth_id(), "original-authorization");
    assert_eq!(transport.side_effects.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn client_authorization_tag_is_stable_and_tracks_the_home_key() {
    let session = Arc::new(MemorySession::default());
    let dc_id = session.home_dc_id().unwrap();
    let pool = SenderPool::new(session.clone(), 1);
    let client = Client::new(pool.handle.clone());
    assert!(client.authorization_key_tag().unwrap().is_none());
    let mut option = session.dc_option(dc_id).unwrap().unwrap();
    option.auth_key = Some([0x31; 256]);
    session.set_dc_option(&option).await.unwrap();
    let original = client.authorization_key_tag().unwrap().unwrap();
    assert_eq!(original.0, dc_id);
    assert_eq!(
        Some(original),
        client.clone().authorization_key_tag().unwrap()
    );

    option.auth_key.as_mut().unwrap()[255] ^= 1;
    session.set_dc_option(&option).await.unwrap();
    assert_ne!(Some(original), client.authorization_key_tag().unwrap());

    option.auth_key = Some([0x31; 256]);
    option.id = if dc_id == 2 { 3 } else { 2 };
    session.set_dc_option(&option).await.unwrap();
    session.set_home_dc_id(option.id).await.unwrap();
    let migrated = client.authorization_key_tag().unwrap().unwrap();
    assert_ne!(migrated.0, original.0);
    assert_eq!(migrated.1, original.1);

    option.auth_key = None;
    session.set_dc_option(&option).await.unwrap();
    assert!(client.authorization_key_tag().unwrap().is_none());
}
