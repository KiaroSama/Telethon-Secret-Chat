#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

use grammers_tl_types as tl;
use kiaro_secret_chat::{
    enums,
    error::Error,
    manager::{Event, Options, SendFileOptions, SendMessageOptions, StrongSecretChatManager},
    media::{FileSource, MediaReference},
};
use std::{sync::mpsc, time::Duration};

#[path = "support_manager/bounded.rs"]
mod bounded;
#[allow(dead_code)]
#[path = "support_manager/mod.rs"]
mod support;

#[tokio::test]
async fn rich_text_and_reference_media_round_trip() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let (sender, receiver, world, id) = support::established().await;
        let (events, inbox) = mpsc::channel();
        receiver
            .on("MessageReceived", "fixture", move |event| {
                if let Event::MessageReceived(message) = event {
                    events.send(message).map_err(|_| Error::Transport)?;
                }
                Ok(())
            })
            .unwrap();

        let folder = tempfile::tempdir().unwrap();
        sender
            .start_auto_save_secret_chats(folder.path())
            .await
            .unwrap();
        assert!(sender.auto_save_secret_chats().unwrap().is_some());
        sender.set_ttl(id, 45).await.unwrap();
        let sends = world.lock().unwrap().sent.len();
        assert!(matches!(
            sender
                .send_message_with_options(
                    id,
                    "invalid TTL",
                    SendMessageOptions {
                        ttl: Some(-1),
                        ..Default::default()
                    }
                )
                .await,
            Err(Error::MediaField { field: "ttl" })
        ));
        assert_eq!(world.lock().unwrap().sent.len(), sends);
        let text_id = sender
            .send_message_with_options(
                id,
                "fixture",
                SendMessageOptions {
                    entities: Some(vec![
                        tl::types::MessageEntityBold {
                            offset: 0,
                            length: 7,
                        }
                        .into(),
                    ]),
                    reply_to_random_id: Some(123),
                    ttl: Some(20),
                    silent: true,
                },
            )
            .await
            .unwrap();
        let outgoing = world.lock().unwrap().sent.last().unwrap().clone();
        let enums::DecryptedMessage::Message(body) =
            kiaro_secret_chat::framing::unwrap(&outgoing.outgoing.body)
                .unwrap()
                .message
        else {
            panic!("expected text frame");
        };
        assert!(body.silent);
        assert_eq!(body.ttl, 20);
        assert_eq!(sender.status(id).unwrap().ttl, 45);
        assert!(!sender.read_saved_messages(id).unwrap().is_empty());
        support::pump(&sender, &receiver, &world).await;
        receiver.settle().await.unwrap();
        let received = inbox.try_recv().unwrap();
        assert_eq!(
            (
                received.random_id,
                received.text.as_str(),
                received.reply_to,
                received.ttl
            ),
            (text_id, "fixture", Some(123), 20)
        );
        assert!(matches!(
            received.entities.as_deref(),
            Some([enums::MessageEntity::Bold(_)])
        ));

        let bytes = b"parity fixture";
        let media_id = sender
            .send_file(
                id,
                FileSource::Bytes {
                    data: bytes.to_vec(),
                    name: "fixture.bin".into(),
                },
                SendFileOptions {
                    caption: "fixture caption".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        support::pump(&sender, &receiver, &world).await;
        receiver.settle().await.unwrap();
        let media = inbox.try_recv().unwrap();
        assert_eq!(media.random_id, media_id);
        let reference = media.media_reference.as_ref().unwrap();
        assert_eq!(receiver.receive_file(reference).await.unwrap(), bytes);
        assert!(
            receiver
                .receive_file_with_limit(reference, bytes.len() - 1)
                .await
                .is_err()
        );
        let destination = folder.path().join("explicit destination.bin");
        let saved = receiver.save_file(reference, &destination).await.unwrap();
        assert_eq!(std::fs::read(saved).unwrap(), bytes);
        receiver
            .forward_file(id, reference, "forwarded", Some(media_id))
            .await
            .unwrap();
        assert_eq!(world.lock().unwrap().uploads, 1);

        sender.set_accepting_requests(false).unwrap();
        assert!(matches!(
            sender.create(support::peer(2000)).await,
            Err(Error::Paused)
        ));
        assert!(!sender.is_suspended(id).unwrap());
        sender.set_typing(id, None).await.unwrap();
        sender.delete_messages(id, vec![text_id]).await.unwrap();
        sender.screenshot(id, vec![media_id]).await.unwrap();
        sender.flush_history(id).await.unwrap();
        sender.stop_auto_save_secret_chats().await.unwrap();
        assert!(sender.auto_save_secret_chats().unwrap().is_none());
        sender.delete_saved_messages(id).unwrap();
        assert!(sender.read_saved_messages(id).unwrap().is_empty());
        sender.stop().await.unwrap();
        receiver.stop().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bounded_pause_blocks_send_until_custody_resumes() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let storage = support::TestStorage::default();
        let (manager, peer, _, id) =
            bounded::pair(storage, bounded::limits(), Options::default()).await;
        manager.pause_chat(id).unwrap();
        assert!(manager.pause_status(id).unwrap().paused);
        assert!(
            manager
                .send_message(id, "blocked", None, None)
                .await
                .is_err()
        );
        assert!(manager.check_custody().unwrap().is_empty());
        manager.resume_chat(id).await.unwrap();
        assert!(!manager.pause_status(id).unwrap().paused);
        manager
            .send_message(id, "resumed", None, None)
            .await
            .unwrap();
        manager.stop().await.unwrap();
        peer.stop().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn strong_ingress_refuses_an_unproved_client_without_network() {
    use grammers_mtsender::SenderPool;
    use grammers_session::{Session, storages::MemorySession};
    use kiaro_secret_chat::{
        binding::AuthorizationBinding,
        custody::IngressLimits,
        ingress::{DurableIngress, EncryptedIngressStorage},
        transport::GrammersTransport,
    };
    use std::sync::Arc;

    let session = Arc::new(MemorySession::default());
    let dc_id = session.home_dc_id().unwrap();
    let mut dc = session.dc_option(dc_id).unwrap().unwrap();
    dc.auth_key = Some([0x71; 256]);
    session.set_dc_option(&dc).await.unwrap();
    let pool = SenderPool::new(session, 1);
    let client = grammers_client::Client::new(pool.handle.clone());
    let (_, key_tag) = client.authorization_key_tag().unwrap().unwrap();
    let binding = AuthorizationBinding::v2(123, dc_id, key_tag, "fixture").unwrap();
    let folder = tempfile::tempdir().unwrap();
    let ingress = DurableIngress::new(
        EncryptedIngressStorage::open(
            folder.path().join("ingress.bin"),
            [9; 32],
            binding,
            1024 * 1024,
        )
        .unwrap(),
        IngressLimits::new(32, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let adapter = GrammersTransport::with_durable_ingress(client, Arc::clone(&ingress));
    assert!(matches!(
        adapter.require_strong_ingress(),
        Err(Error::IngressUnavailable)
    ));
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        StrongSecretChatManager::with_transport(adapter, ingress, bounded::limits()),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(Error::IngressUnavailable)));
}

#[allow(dead_code)]
async fn strong_facade_has_the_typed_contract<T: kiaro_secret_chat::manager::Transport>(
    manager: &StrongSecretChatManager<T>,
    update: &tl::enums::Update,
    reference: &MediaReference,
    path: &std::path::Path,
) -> kiaro_secret_chat::error::Result<()> {
    manager.on("ChatClosed", "fixture", |event: Event| {
        assert!(matches!(event, Event::ChatClosed { .. }));
        Ok(())
    })?;
    manager.handle_update(update).await?;
    manager.drain_ingress().await?;
    manager
        .send_message_with_options(7, "text", SendMessageOptions::default())
        .await?;
    manager
        .send_file(
            7,
            FileSource::Bytes {
                data: vec![1],
                name: "fixture.bin".into(),
            },
            SendFileOptions::default(),
        )
        .await?;
    manager.forward_file(7, reference, "", None).await?;
    manager.receive_file(reference).await?;
    manager.save_file(reference, path).await?;
    manager.pause_chat(7)?;
    manager.resume_chat(7).await?;
    manager.settle().await?;
    assert!(matches!(
        manager.start_auto_save_secret_chats(path).await,
        Err(Error::IngressUnavailable)
    ));
    Ok(())
}
