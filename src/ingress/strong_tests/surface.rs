//! Public Strong facade exercised against its real durable protocol and file pipeline.
use super::*;
use crate::{
    error::Error,
    manager::{Event, MessageReceived, SendFileOptions},
    media::{FileSource, MediaReference},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(super) async fn forwarded_operations_preserve_durable_and_file_contracts() {
    let f = started().await;
    let created = f.manager.create(fake::peer(2000)).await.unwrap();
    assert_eq!(created.state, crate::chat::ChatState::Requested);
    assert_eq!(f.manager.list().unwrap().len(), 1);
    assert_eq!(f.manager.status(created.id).unwrap(), created);
    install(&f, 8);
    let dh = crate::dh::DhConfig::new(2, &fake::prime()).unwrap();
    let secret = crate::dh::SecretExponent::generate().unwrap();
    let public = dh.public_value(&secret);
    f.world
        .lock()
        .unwrap()
        .requests
        .insert(9, (2000, 1000, public.clone()));
    capture(
        &f,
        vec![
            tl::types::UpdateEncryption {
                chat: tl::types::EncryptedChatRequested {
                    folder_id: None,
                    id: 9,
                    access_hash: 88,
                    date: 124,
                    admin_id: 2000,
                    participant_id: 1000,
                    g_a: public,
                }
                .into(),
                date: 124,
            }
            .into(),
        ],
        2,
    )
    .await;
    f.manager.drain_ingress().await.unwrap();
    assert_eq!(
        f.manager.status(9).unwrap().state,
        crate::chat::ChatState::Waiting
    );
    assert_eq!(
        f.manager.accept(9).await.unwrap().state,
        crate::chat::ChatState::Ready
    );
    let random = f
        .manager
        .send_message(8, "facade **formatted**", None, None)
        .await
        .unwrap();
    let sent = f.world.lock().unwrap().sent.last().unwrap().clone();
    let crate::enums::DecryptedMessage::Message(message) =
        crate::framing::unwrap(&sent.outgoing.body).unwrap().message
    else {
        panic!("text message")
    };
    assert_eq!(message.message, "facade formatted");
    assert_eq!(message.random_id, random);
    assert!(
        message
            .entities
            .unwrap()
            .iter()
            .any(|entity| matches!(entity, crate::enums::MessageEntity::Bold(_)))
    );
    f.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
    assert!(matches!(
        f.manager
            .send_message(8, "uncertain facade", Some(vec![]), None)
            .await,
        Err(Error::SendPending { .. })
    ));
    let uncertain = f.world.lock().unwrap().sent.last().unwrap().clone();
    f.manager.retry_pending(8).await.unwrap();
    let repeated = f.world.lock().unwrap().sent.last().unwrap().clone();
    assert_eq!(uncertain.outgoing.frame, repeated.outgoing.frame);
    assert_eq!(uncertain.outgoing.random_id, repeated.outgoing.random_id);
    f.manager.set_ttl(8, 42).await.unwrap();
    assert_eq!(f.manager.status(8).unwrap().ttl, 42);
    f.manager.mark_read(8, vec![random]).await.unwrap();
    f.manager.screenshot(8, vec![random]).await.unwrap();
    f.manager.set_typing(8, None).await.unwrap();
    f.manager
        .set_typing(
            8,
            Some(crate::enums::SendMessageAction::SendMessageCancelAction),
        )
        .await
        .unwrap();
    f.manager.rekey(8).await.unwrap();
    assert!(f.manager.status(8).unwrap().exchange_in_progress);
    assert!(f.manager.check_custody().unwrap().is_empty());

    let got = Arc::new(Mutex::new(Vec::<MessageReceived>::new()));
    let seen = Arc::clone(&got);
    f.manager
        .on("MessageReceived", "facade_collect", move |event| {
            if let Event::MessageReceived(message) = event {
                seen.lock().unwrap().push(*message);
            }
            Ok(())
        })
        .unwrap();
    let completed = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&completed);
    f.manager
        .on_async("MessageReceived", "facade_future", move |_| {
            let observed = Arc::clone(&observed);
            async move {
                observed.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
        })
        .unwrap();
    let observed = Arc::clone(&completed);
    f.manager
        .on_async_with_cleanup(
            "MessageReceived",
            "facade_cleanup",
            move |_| {
                let observed = Arc::clone(&observed);
                async move {
                    observed.fetch_add(1, Ordering::AcqRel);
                    Ok(())
                }
            },
            |_| async { Ok(()) },
        )
        .unwrap();
    let content = b"explicit strong file bytes".repeat(20);
    f.manager
        .send_file(
            8,
            FileSource::bytes(content.clone(), "facade.bin").unwrap(),
            SendFileOptions::default(),
        )
        .await
        .unwrap();
    let effect = f.world.lock().unwrap().sent.last().unwrap().clone();
    let crate::enums::DecryptedMessage::Message(message) =
        crate::framing::unwrap(&effect.outgoing.body)
            .unwrap()
            .message
    else {
        panic!("file message")
    };
    let media = message.media.unwrap();
    let (file_id, fingerprint, encrypted_len) = {
        let world = f.world.lock().unwrap();
        let file_id = *world.files.keys().next().unwrap();
        (
            file_id,
            world.fingerprints[&file_id],
            world.files[&file_id].len(),
        )
    };
    let file = tl::types::EncryptedFile {
        id: file_id,
        access_hash: 99,
        size: encrypted_len as i64,
        dc_id: 1,
        key_fingerprint: fingerprint,
    };
    let reference = MediaReference::new(8, &file, &media).unwrap();
    let inbound = crate::types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 201,
        ttl: 0,
        message: "peer file".into(),
        media: Some(media),
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    };
    let incoming: tl::enums::Update = tl::types::UpdateNewEncryptedMessage {
        message: tl::types::EncryptedMessage {
            random_id: 201,
            chat_id: 8,
            date: 124,
            bytes: encrypt(inbound.into(), 0),
            file: file.into(),
        }
        .into(),
        qts: 41,
    }
    .into();
    capture(&f, vec![incoming], 3).await;
    f.manager.drain_ingress().await.unwrap();
    f.manager.settle().await.unwrap();
    assert_eq!(completed.load(Ordering::Acquire), 2);
    let received = got.lock().unwrap()[0].clone();
    assert_eq!(received.media_reference.as_ref(), Some(&reference));
    assert!(f.manager.read_history(8, 100).unwrap().is_empty());
    assert_eq!(f.manager.pending_deliveries(8).unwrap().len(), 1);
    assert_eq!(f.manager.receive_file(&reference).await.unwrap(), content);
    assert_eq!(
        f.manager
            .receive_file_with_limit(&reference, content.len())
            .await
            .unwrap(),
        content
    );
    assert!(
        f.manager
            .receive_file_with_limit(&reference, content.len() - 1)
            .await
            .is_err()
    );
    assert_eq!(
        f.manager.receive_message_file(&received).await.unwrap(),
        content
    );
    let output = f.dir.path().join("explicit-files");
    let direct = output.join("direct.bin");
    let from_message = output.join("message.bin");
    f.manager.save_file(&reference, &direct).await.unwrap();
    f.manager
        .save_message_file(&received, &from_message)
        .await
        .unwrap();
    assert_eq!(std::fs::read(direct).unwrap(), content);
    assert_eq!(std::fs::read(from_message).unwrap(), content);
    let (uploads, downloads) = {
        let world = f.world.lock().unwrap();
        (world.uploads, world.downloads)
    };
    f.manager
        .forward_file(8, &reference, "forwarded", Some(201))
        .await
        .unwrap();
    f.manager
        .forward_message_file(8, &received, "message forward", None)
        .await
        .unwrap();
    {
        let world = f.world.lock().unwrap();
        assert_eq!(world.uploads, uploads);
        assert_eq!(world.downloads, downloads);
    }
    f.manager.acknowledge_delivery(8, received.seq_no).unwrap();
    assert!(f.manager.pending_deliveries(8).unwrap().is_empty());
    assert!(
        f.manager
            .start_auto_save_secret_chats(&output)
            .await
            .is_err()
    );
    assert!(f.manager.auto_save_secret_chats().is_none());
    f.manager.delete_messages(8, [random]).await.unwrap();
    f.manager.flush_history(8).await.unwrap();
    f.manager.close(8).await.unwrap();
    assert!(f.manager.pause_status(8).unwrap().paused);
    assert!(!f.manager.pause_status(8).unwrap().resumable);
    assert!(f.world.lock().unwrap().discards.is_empty());
    f.manager.stop().await.unwrap();
    assert!(
        f.manager
            .send_message(8, "after stop", None, None)
            .await
            .is_err()
    );
    assert!(f.manager.retry_pending(8).await.is_err());
    assert!(f.manager.receive_file(&reference).await.is_err());
    assert!(f.manager.accept(9).await.is_err());
    assert!(f.manager.close(8).await.is_err());
}
