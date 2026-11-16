//! Exercise production adapters through an original Client and typed offline RPCs.
#[path = "review_custody.rs"]
mod custody;
#[path = "adapter_export.rs"]
mod export;
#[path = "adapter_media.rs"]
mod media;
#[path = "adapter_server.rs"]
pub mod server;
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord},
    crypto::SharedKey,
    enums,
    grammers_client::Client,
    grammers_mtsender::SenderPool,
    grammers_session::{SessionData, storages::MemorySession},
    grammers_tl_types as tl,
    manager::{RpcFailure, Transport},
    protocol::Protocol,
    storage::MemoryStorage,
    transport::GrammersTransport,
};
use std::sync::{Arc, Mutex};
pub async fn run() {
    Box::pin(custody::run()).await;
    let mut session = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    for dc in [2, 4] {
        session.dc_options.get_mut(&dc).unwrap().auth_key = Some([7; 256]);
    }
    let pool = SenderPool::new(Arc::new(MemorySession::from(session)), 123);
    let client = Client::new(pool.handle.clone());
    let handle = pool.handle.clone();
    let model = Arc::new(Mutex::new(server::Server {
        discard_ok: true,
        ..Default::default()
    }));
    let remote = model.clone();
    let runner = tokio::spawn(pool.runner.run_offline_fixture(move |dc, bytes| {
        let remote = remote.clone();
        async move { remote.lock().unwrap().invoke(dc, bytes) }
    }));
    let transport = GrammersTransport::new(client.clone());
    assert!(Transport::requires_verified_binding(&transport));
    assert!(transport.require_strong_ingress().is_err());
    let binding = transport.verified_binding("fixture").await.unwrap();
    assert_eq!((binding.user_id(), binding.dc_id()), (123, 2));
    assert!(
        Transport::authorization_digest(&transport)
            .await
            .unwrap()
            .is_some()
    );
    for mode in [1, 2, 3, 4] {
        model.lock().unwrap().binding_mode = mode;
        assert!(transport.verified_binding("fixture").await.is_err());
    }
    model.lock().unwrap().binding_mode = 0;
    model.lock().unwrap().failure = Some("USER_DELETED");
    assert!(transport.verified_binding("fixture").await.is_err());
    transport.dh_config().await.unwrap();
    Transport::dh_config(&transport).await.unwrap();
    model.lock().unwrap().dh_mode = 1;
    assert!(transport.dh_config().await.is_err());
    model.lock().unwrap().dh_mode = 0;
    model.lock().unwrap().failure = Some("DATA_INVALID");
    assert!(transport.dh_config().await.is_err());
    let user: tl::enums::InputUser = tl::types::InputUser {
        user_id: 456,
        access_hash: 99,
    }
    .into();
    assert!(matches!(
        transport
            .request_encryption(user.clone(), 77, vec![2; 256])
            .await
            .unwrap(),
        tl::enums::EncryptedChat::Waiting(_)
    ));
    Transport::request(&transport, user.clone(), 77, vec![2; 256])
        .await
        .unwrap();
    for name in ["ENCRYPTION_ID_INVALID", "DATA_INVALID", "OTHER_REJECTION"] {
        model.lock().unwrap().failure = Some(name);
        assert!(
            Transport::request(&transport, user.clone(), 77, vec![2; 256])
                .await
                .is_err()
        );
    }
    transport
        .accept_encryption(7, 88, vec![3; 256], 123)
        .await
        .unwrap();
    Transport::accept(&transport, 7, 88, vec![3; 256], 123)
        .await
        .unwrap();
    model.lock().unwrap().failure = Some("ENCRYPTION_ALREADY_ACCEPTED");
    assert!(matches!(
        Transport::accept(&transport, 7, 88, vec![3; 256], 123).await,
        Err(RpcFailure::AlreadyAccepted)
    ));
    model.lock().unwrap().failure = Some("ENCRYPTION_ALREADY_DECLINED");
    assert!(
        transport
            .accept_encryption(7, 88, vec![3; 256], 123)
            .await
            .is_err()
    );
    let mut p = Protocol::open(MemoryStorage::default(), "11".repeat(32)).unwrap();
    p.start().unwrap();
    p.establish(
        Chat::new(7, 88, 456, true, ChatState::Waiting),
        SharedKey::from_slice(&[7; 256]).unwrap(),
        DhRecord {
            g: 2,
            prime: server::hex_prime(),
        },
        1,
    )
    .unwrap();
    let ordinary = p
        .send_message(7, "public native adapter fixture".into(), None, None)
        .unwrap();
    let service = p
        .send_action(7, enums::DecryptedMessageAction::Noop)
        .unwrap();
    let mut file = ordinary.clone();
    file.outgoing.file = Some(
        tl::enums::InputEncryptedFile::from(tl::types::InputEncryptedFile {
            id: 9,
            access_hash: 99,
        })
        .to_bytes(),
    );
    for effect in [&ordinary, &service, &file] {
        transport.send(effect).await.unwrap();
        let response = Transport::send(&transport, effect).await.unwrap();
        assert_eq!(response.file.is_some(), effect.outgoing.file.is_some());
    }
    assert_eq!(model.lock().unwrap().sent.len(), 6);
    for (index, effect) in [&ordinary, &service, &file].iter().enumerate() {
        for call in &model.lock().unwrap().sent[2 * index..2 * index + 2] {
            assert_eq!(call.1, effect.outgoing.random_id);
            assert_eq!(call.2, effect.outgoing.frame);
        }
    }
    let mut bad = file.clone();
    bad.outgoing.file = Some(vec![1, 2, 3]);
    assert!(transport.send(&bad).await.is_err());
    assert!(matches!(
        Transport::send(&transport, &bad).await,
        Err(RpcFailure::Permanent)
    ));
    model.lock().unwrap().failure = Some("ENCRYPTION_DECLINED");
    assert!(matches!(
        Transport::send(&transport, &ordinary).await,
        Err(RpcFailure::ChatEnded)
    ));
    model.lock().unwrap().failure = Some("DATA_INVALID");
    assert!(transport.send(&ordinary).await.is_err());
    media::run(&transport, &model).await;
    export::run(client.clone(), &model).await;
    transport.discard(7, true).await.unwrap();
    Transport::discard(&transport, 7, false).await.unwrap();
    model.lock().unwrap().discard_ok = false;
    assert!(transport.discard(7, false).await.is_err());
    model.lock().unwrap().failure = Some("ENCRYPTION_ID_INVALID");
    assert!(transport.discard(7, false).await.is_err());
    handle.quit();
    runner.await.unwrap();
    println!("Native adapter request, encryption and file boundaries verified");
}
