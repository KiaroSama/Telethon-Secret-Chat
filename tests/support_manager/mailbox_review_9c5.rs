use super::*;
pub(super) async fn concurrent_snapshot_keeps_flush_after_ordinary() {
    let (manager, _, _, id) = seeded::ready(Options::default()).await;
    let observed = Arc::new(Mutex::new(Vec::new()));
    for name in ["MessageReceived", "ServiceActionReceived"] {
        let observed = observed.clone();
        manager
            .on(name, name, move |event| {
                observed.lock().unwrap().push(match event {
                    Event::MessageReceived(_) => "message",
                    Event::ServiceActionReceived { .. } => "flush",
                    _ => unreachable!(),
                });
                Ok(())
            })
            .unwrap();
    }
    for pair in 0..128 {
        let raw = pair * 2;
        let ordinary = encrypt_frame(
            &SharedKey::from_slice(&[7; 256]).unwrap(),
            &body("before concurrent flush", raw),
            Direction::Responder,
        )
        .unwrap();
        let service = crate::framing::wrap(
            crate::types::DecryptedMessageService {
                random_id: 1000 + i64::from(pair),
                action: crate::enums::DecryptedMessageAction::FlushHistory,
            }
            .into(),
            144,
            1,
            (raw + 1) * 2,
        )
        .unwrap()
        .to_bytes();
        let flush = encrypt_frame(
            &SharedKey::from_slice(&[7; 256]).unwrap(),
            &service,
            Direction::Responder,
        )
        .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writer = manager.clone();
        let start = barrier.clone();
        let thread = std::thread::spawn(move || {
            start.wait();
            writer
                .protocol(|protocol| {
                    protocol.receive(id, &ordinary, None, 1)?;
                    protocol.receive(id, &flush, None, 2)?;
                    Ok(())
                })
                .unwrap();
        });
        barrier.wait();
        manager.drain_deliveries(id).await.unwrap();
        thread.join().unwrap();
        manager.drain_deliveries(id).await.unwrap();
        assert_eq!(
            *observed.lock().unwrap(),
            ["message", "flush"],
            "one mailbox snapshot must not apply a later flush before its ordinary predecessor"
        );
        observed.lock().unwrap().clear();
        assert!(manager.read_history(id, 10).unwrap().is_empty());
    }
    manager.stop().await.unwrap();
}
