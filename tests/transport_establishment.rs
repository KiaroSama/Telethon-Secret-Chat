#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use kiaro_secret_chat::errors::SecretChatError;
use kiaro_secret_chat::establishment::{SecretChatEstablisher, render_key_visualization_png};
use kiaro_secret_chat::grammers_tl_types as tl;
use kiaro_secret_chat::manager::{BoxFuture, SecretTransport};
use kiaro_secret_chat::protocol::{ChatRole, ChatState};
use kiaro_secret_chat::transport::{
    DhConfigData, EncryptedChatWire, EncryptedFileRef, EncryptedFileUpload, EstablishmentTransport,
    GrammersTransport, IncomingWireUpdate,
};

const CHAT: i32 = -7;
const HASH: i64 = 970;
const ALICE: i64 = 10;
const BOB: i64 = 20;

fn config() -> DhConfigData {
    // RFC 3526, group 14.
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
    .expect("decode RFC 3526 prime");
    DhConfigData {
        g: 2,
        p,
        version: 1,
        random: vec![0x6d; 256],
    }
}

type Rpc<'a, T> = BoxFuture<'a, Result<T, SecretChatError>>;

#[derive(Default)]
struct Server {
    requested: Option<EncryptedChatWire>,
    active: Option<EncryptedChatWire>,
    discarded: Vec<(i32, bool)>,
    frames: Vec<(i64, i64, Vec<u8>)>,
    upload: Vec<u8>,
    tamper_accept: bool,
}

#[derive(Clone)]
struct MockEstablishmentTransport {
    identity: &'static str,
    server: Arc<Mutex<Server>>,
    dh: DhConfigData,
}

impl MockEstablishmentTransport {
    fn pair() -> (Self, Self) {
        let alice = Self {
            identity: "alice-test",
            server: Arc::default(),
            dh: config(),
        };
        let bob = Self {
            identity: "bob-test",
            ..alice.clone()
        };
        (alice, bob)
    }
}

impl SecretTransport for MockEstablishmentTransport {
    fn auth_identity(&self) -> &str {
        self.identity
    }

    fn send_encrypted(&self, chat_id: i64, random_id: i64, frame: Vec<u8>) -> Rpc<'_, ()> {
        Box::pin(async move {
            self.server
                .lock()
                .unwrap()
                .frames
                .push((chat_id, random_id, frame));
            Ok(())
        })
    }

    fn send_encrypted_service(&self, chat_id: i64, random_id: i64, frame: Vec<u8>) -> Rpc<'_, ()> {
        self.send_encrypted(chat_id, random_id, frame)
    }

    fn discard_encryption(&self, chat_id: i64, delete_history: bool) -> Rpc<'_, ()> {
        Box::pin(async move {
            EstablishmentTransport::discard_encryption(
                self,
                i32::try_from(chat_id).unwrap(),
                delete_history,
            )
            .await?;
            Ok(())
        })
    }
}

impl EstablishmentTransport for MockEstablishmentTransport {
    fn get_dh_config(&self, version: i32, random_length: i32) -> Rpc<'_, DhConfigData> {
        Box::pin(async move {
            assert_eq!((version, random_length), (0, 256));
            Ok(self.dh.clone())
        })
    }

    fn request_encryption(
        &self,
        peer_user_id: i64,
        peer_access_hash: i64,
        _: i32,
        g_a: Vec<u8>,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            assert_eq!((peer_user_id, peer_access_hash), (BOB, 25));
            self.server.lock().unwrap().requested = Some(requested(g_a));
            Ok(EncryptedChatWire::Waiting {
                chat_id: CHAT,
                access_hash: HASH,
                admin_id: ALICE,
                participant_id: BOB,
                date: 1,
            })
        })
    }

    fn accept_encryption(
        &self,
        chat_id: i32,
        access_hash: i64,
        g_b: Vec<u8>,
        key_fingerprint: i64,
    ) -> Rpc<'_, EncryptedChatWire> {
        Box::pin(async move {
            assert_eq!((chat_id, access_hash), (CHAT, HASH));
            let mut server = self.server.lock().unwrap();
            let Some(EncryptedChatWire::Requested { g_a, .. }) = &server.requested else {
                panic!("mock expected a prior request");
            };
            let response = EncryptedChatWire::Active {
                chat_id: CHAT,
                access_hash: HASH,
                admin_id: ALICE,
                participant_id: BOB,
                g_a_or_b: g_a.clone(),
                key_fingerprint: key_fingerprint ^ i64::from(server.tamper_accept),
                date: 2,
            };
            server.active = Some(EncryptedChatWire::Active {
                chat_id: CHAT,
                access_hash: HASH,
                admin_id: ALICE,
                participant_id: BOB,
                g_a_or_b: g_b,
                key_fingerprint,
                date: 2,
            });
            Ok(response)
        })
    }

    fn discard_encryption(&self, chat_id: i32, delete_history: bool) -> Rpc<'_, bool> {
        Box::pin(async move {
            self.server
                .lock()
                .unwrap()
                .discarded
                .push((chat_id, delete_history));
            Ok(true)
        })
    }

    fn send_encrypted_file(
        &self,
        chat_id: i32,
        access_hash: i64,
        random_id: i64,
        data: Vec<u8>,
        _: EncryptedFileUpload,
    ) -> Rpc<'_, i32> {
        Box::pin(async move {
            assert_eq!(access_hash, HASH);
            self.send_encrypted(i64::from(chat_id), random_id, data)
                .await?;
            Ok(3)
        })
    }

    fn upload_encrypted_bytes(
        &self,
        ciphertext: Vec<u8>,
        key_fingerprint: i32,
    ) -> Rpc<'_, EncryptedFileUpload> {
        Box::pin(async move {
            self.server.lock().unwrap().upload = ciphertext;
            Ok(EncryptedFileUpload {
                id: 1,
                parts: 1,
                md5_checksum: String::new(),
                key_fingerprint,
                is_big: false,
            })
        })
    }

    fn download_encrypted_bytes(&self, file: &EncryptedFileRef) -> Rpc<'_, Vec<u8>> {
        let size = file.size;
        Box::pin(async move {
            let bytes = self.server.lock().unwrap().upload.clone();
            assert_eq!(usize::try_from(size).unwrap(), bytes.len());
            Ok(bytes)
        })
    }
}

fn requested(g_a: Vec<u8>) -> EncryptedChatWire {
    EncryptedChatWire::Requested {
        chat_id: CHAT,
        access_hash: HASH,
        admin_id: ALICE,
        participant_id: BOB,
        g_a,
        date: 1,
    }
}

#[tokio::test]
async fn alice_and_bob_establish_and_exchange_messages() -> Result<(), SecretChatError> {
    let (alice, bob) = MockEstablishmentTransport::pair();
    let initiator = SecretChatEstablisher::new(alice.clone());
    let acceptor = SecretChatEstablisher::new(bob);
    let pending = initiator.request_chat(BOB, 25).await?;
    let offered = alice.server.lock().unwrap().requested.clone().unwrap();
    let mut bob = acceptor.accept_chat(&offered).await?;
    let active = alice.server.lock().unwrap().active.clone().unwrap();
    let mut alice = initiator.complete_outgoing_handshake(pending, &active)?;

    assert_eq!(alice.chat.role(), ChatRole::Initiator);
    assert_eq!(bob.chat.role(), ChatRole::Acceptor);
    assert_eq!(alice.chat.state(), ChatState::Ready);
    assert_eq!(bob.chat.state(), ChatState::Ready);
    assert_eq!(alice.chat.key_lifecycle(), bob.chat.key_lifecycle());
    assert_eq!(alice.g_a, bob.g_a);
    assert_eq!(alice.key_picture, bob.key_picture);
    assert_eq!(alice.key_picture.hex.len(), 64);

    let frame = alice.encrypt_text(1, "hello Bob", 0)?;
    let received = bob.chat.receive_encrypted_frame(&frame)?;
    assert_eq!(received.text.as_deref(), Some("hello Bob"));
    assert_eq!(received.random_id, 1);
    let frame = bob.encrypt_text(2, "hello Alice", 0)?;
    let received = alice.chat.receive_encrypted_frame(&frame)?;
    assert_eq!(received.text.as_deref(), Some("hello Alice"));
    assert_eq!(received.random_id, 2);
    Ok(())
}

#[tokio::test]
async fn refuses_invalid_dh_configs_and_peer_values() -> Result<(), SecretChatError> {
    let (alice, bob) = MockEstablishmentTransport::pair();
    for dh in [
        DhConfigData { g: 1, ..config() },
        DhConfigData {
            p: vec![17],
            ..config()
        },
        DhConfigData {
            random: vec![0; 255],
            ..config()
        },
    ] {
        let transport = MockEstablishmentTransport {
            dh,
            ..alice.clone()
        };
        assert!(matches!(
            SecretChatEstablisher::new(transport)
                .request_chat(BOB, 25)
                .await,
            Err(SecretChatError::ParameterRejected { .. })
        ));
    }
    assert!(alice.server.lock().unwrap().requested.is_none());

    let mut p_minus_one = config().p;
    *p_minus_one.last_mut().unwrap() -= 1;
    let acceptor = SecretChatEstablisher::new(bob);
    let initiator = SecretChatEstablisher::new(alice);
    for peer_value in [vec![0], vec![1], p_minus_one, config().p] {
        assert!(matches!(
            acceptor.accept_chat(&requested(peer_value.clone())).await,
            Err(SecretChatError::ParameterRejected { .. })
        ));
        let pending = initiator.request_chat(BOB, 25).await?;
        let active = EncryptedChatWire::Active {
            chat_id: CHAT,
            access_hash: HASH,
            admin_id: ALICE,
            participant_id: BOB,
            g_a_or_b: peer_value,
            key_fingerprint: 0,
            date: 2,
        };
        assert!(matches!(
            initiator.complete_outgoing_handshake(pending, &active),
            Err(SecretChatError::ParameterRejected { .. })
        ));
    }
    Ok(())
}

#[tokio::test]
async fn refuses_tampered_fingerprints_and_propagates_discard_flags() -> Result<(), SecretChatError>
{
    let (alice, bob) = MockEstablishmentTransport::pair();
    let initiator = SecretChatEstablisher::new(alice.clone());
    let acceptor = SecretChatEstablisher::new(bob);
    let pending = initiator.request_chat(BOB, 25).await?;
    let offered = alice.server.lock().unwrap().requested.clone().unwrap();
    alice.server.lock().unwrap().tamper_accept = true;
    assert!(matches!(
        acceptor.accept_chat(&offered).await,
        Err(SecretChatError::ParameterRejected { .. })
    ));

    let mut active = alice.server.lock().unwrap().active.clone().unwrap();
    if let EncryptedChatWire::Active {
        key_fingerprint, ..
    } = &mut active
    {
        *key_fingerprint ^= 1;
    }
    assert!(matches!(
        initiator.complete_outgoing_handshake(pending, &active),
        Err(SecretChatError::ParameterRejected { .. })
    ));
    assert!(initiator.discard_chat(CHAT, false).await?);
    assert!(acceptor.delete_chat_both_sides(CHAT).await?);
    assert_eq!(
        alice.server.lock().unwrap().discarded,
        [(CHAT, false), (CHAT, true)]
    );
    Ok(())
}

#[test]
fn png_has_official_pair_order_and_stored_image_data() -> Result<(), SecretChatError> {
    let (picture, png) = render_key_visualization_png(&[0; 256], &[2], 2)?;
    // SHA1(256 zero bytes)[..16] followed by SHA256(256 zero bytes)[..16].
    assert_eq!(
        picture.hex,
        "b376885ac8452b6cbf9ced81b1080bfd5341e6b2646979a70e57653007a1f310"
    );
    assert_eq!(&picture.rows[0][..4], &[3, 0, 3, 2]);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        &png[8..29],
        b"\0\0\0\rIHDR\0\0\0\x18\0\0\0\x18\x08\x02\0\0\0"
    );
    assert_eq!(&png[png.len() - 12..], b"\0\0\0\0IEND\xaeB`\x82");
    assert_eq!(&png[37..41], b"IDAT");
    assert_eq!(&png[41..48], b"\x78\x01\x01\xd8\x06\x27\xf9");
    assert_eq!(&png[48..55], &[0, 47, 153, 201, 47, 153, 201]);
    assert!(render_key_visualization_png(&[0; 256], &[2], 0).is_err());
    assert!(render_key_visualization_png(&[0; 256], &[2], u32::MAX).is_err());
    Ok(())
}

#[test]
fn adapts_layer_234_encryption_and_message_updates() {
    fn assert_grammers_transport<T: EstablishmentTransport + SecretTransport>() {}
    assert_grammers_transport::<GrammersTransport>();

    let update = tl::types::UpdateEncryption {
        chat: tl::types::EncryptedChatRequested {
            folder_id: None,
            id: CHAT,
            access_hash: HASH,
            date: 1,
            admin_id: ALICE,
            participant_id: BOB,
            g_a: vec![9],
        }
        .into(),
        date: 2,
    }
    .into();
    assert_eq!(
        GrammersTransport::adapt_update(&update),
        Some(IncomingWireUpdate::ChatUpdate(requested(vec![9])))
    );

    let file = EncryptedFileRef {
        id: 3,
        access_hash: 4,
        size: 16,
        dc_id: 2,
        key_fingerprint: 6,
    };
    let update = tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessage {
            random_id: 1,
            chat_id: CHAT,
            date: 2,
            bytes: vec![1, 2],
            file: tl::types::EncryptedFile {
                id: 3,
                access_hash: 4,
                size: 16,
                dc_id: 2,
                key_fingerprint: 6,
            }
            .into(),
        }
        .into(),
        qts: 4,
    }
    .into();
    assert_eq!(
        GrammersTransport::adapt_update(&update),
        Some(IncomingWireUpdate::NewEncryptedMessage {
            chat_id: CHAT,
            date: 2,
            bytes: vec![1, 2],
            file: Some(file)
        })
    );

    let update = tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessageService {
            random_id: 2,
            chat_id: CHAT,
            date: 3,
            bytes: vec![3],
        }
        .into(),
        qts: 5,
    }
    .into();
    assert_eq!(
        GrammersTransport::adapt_update(&update),
        Some(IncomingWireUpdate::NewEncryptedMessage {
            chat_id: CHAT,
            date: 3,
            bytes: vec![3],
            file: None
        })
    );
    assert!(GrammersTransport::adapt_update(&tl::types::UpdatePtsChanged {}.into()).is_none());
}
