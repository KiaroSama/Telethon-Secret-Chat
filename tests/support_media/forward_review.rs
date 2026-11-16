use super::*;
pub async fn forward_observes_ttl_after_retry() {
    let (a, b, world, id, message) = sent("payload.bin", None, Metadata::default()).await;
    world.lock().unwrap().fail_send = Some(kiaro_secret_chat::manager::RpcFailure::Transient);
    assert!(
        b.send_message(id, "pending before forward", None, None)
            .await
            .is_err()
    );
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().send_gate = Some(gate.clone());
    let sender = b.clone();
    let reference = message.media_reference.unwrap();
    let forward = tokio::spawn(async move {
        sender
            .forward_file(id, &reference, "updated TTL", None)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    world.lock().unwrap().send_gate = None;
    a.set_ttl(id, 42).await.unwrap();
    let update = world
        .lock()
        .unwrap()
        .updates
        .get_mut(&2000)
        .unwrap()
        .pop_front()
        .unwrap();
    b.handle_update(&update).await;
    assert_eq!(b.status(id).unwrap().ttl, 42);
    gate.release.notify_one();
    let random_id = forward.await.unwrap().unwrap();
    let ttl = world
        .lock()
        .unwrap()
        .sent
        .iter()
        .find(|effect| effect.outgoing.random_id == random_id)
        .map(|effect| {
            match kiaro_secret_chat::framing::unwrap(&effect.outgoing.body)
                .unwrap()
                .message
            {
                enums::DecryptedMessage::Message(message) => message.ttl,
                _ => panic!("ordinary forwarded file"),
            }
        })
        .unwrap();
    a.stop().await.unwrap();
    b.stop().await.unwrap();
    assert_eq!(
        ttl, 42,
        "a forward blocked in retry must read the newly applied chat TTL after that wait"
    );
}
