use super::*;
use kiaro_secret_chat::{
    enums::DecryptedMessageAction,
    manager::{Event, SendFileOptions},
    media::FileSource,
};
pub enum Official {
    Fingerprint,
    RoundTrip,
    Closed,
    MediaOut,
    MediaIn,
    Rekey,
}
pub enum Paired {
    Lifecycle,
    CloseSync,
    CloseSend,
    DeleteBoth,
}
pub async fn official(case: Official) -> TestResult {
    let _exclusive = super::LIVE_RUN.lock().await;
    let config = Config::load("TSC_LIVE_SESSION_A")?;
    let mut account = Account::open(config, false).await?;
    let outcome =
        match tokio::time::timeout(Duration::from_secs(900), official_case(&mut account, case))
            .await
        {
            Ok(result) => result,
            Err(_) => {
                Err(Refused("The explicit official-client scenario exceeded its deadline").into())
            }
        };
    let cleanup = account.shutdown().await;
    outcome.and(cleanup)
}
async fn official_case(a: &mut Account, case: Official) -> TestResult {
    let id = a.ready("official client interoperability").await?;
    match case {
        Official::Fingerprint => {
            let local = a.manager.status(id)?.key_fingerprint;
            let peer = a
                .fingerprints
                .lock()
                .map_err(|_| Refused("The peer fingerprint recorder failed"))?
                .get(&id)
                .copied();
            require(
                local.is_some() && local == peer,
                "The independently reported peer fingerprint does not match the local shared key",
            )?;
        }
        Official::RoundTrip => {
            let code = nonce()?;
            a.manager
                .send_message(id, &format!("interop {code}"), None, None)
                .await?;
            println!(
                "Read the secret message in the official client and REPLY with the code: {code}"
            );
            let message = received(a, id, &code).await?;
            require(
                message.seq_no >= 0 && message.random_id != 0,
                "The official reply has an invalid message identity",
            )?;
            require(
                a.manager
                    .read_history(id, 1000)?
                    .iter()
                    .any(|saved| saved.random_id == message.random_id),
                "The official reply was absent from readable history",
            )?;
        }
        Official::Closed => {
            a.manager.close(id).await?;
            require(
                a.manager.status(id)?.state == ChatState::Closed,
                "Local closure did not become terminal",
            )?;
            require(
                a.manager
                    .send_message(id, "this must not encrypt", None, None)
                    .await
                    .is_err(),
                "A closed secret chat accepted a new send",
            )?;
        }
        Official::MediaOut => media_out(a, id).await?,
        Official::MediaIn => {
            println!(
                "SEND one real file from the official client into this throwaway secret chat."
            );
            let event=a.recorder.wait_for("MessageReceived",HUMAN,|event|matches!(event,Event::MessageReceived(message) if message.chat_id==id&&message.media_reference.is_some()&&message.file.is_some())).await?;
            let Event::MessageReceived(message) = event else {
                return Err(Refused("The official incoming file event is missing").into());
            };
            let reference = message
                .media_reference
                .ok_or(Refused("The official file has no verified media reference"))?;
            let size = reference.validate()?.size;
            let path = a
                .manager
                .save_file(&reference, &a.directory().join("received.bin"))
                .await?;
            require(
                std::fs::metadata(path)
                    .map_err(|_| Refused("The official incoming file was not written"))?
                    .len()
                    == size,
                "The official incoming file size does not match its authenticated metadata",
            )?;
        }
        Official::Rekey => {
            let before = a.manager.status(id)?;
            require(
                before.key_hash.is_some(),
                "The initial visual authentication hash is missing",
            )?;
            a.manager.rekey(id).await?;
            a.recorder.wait_for("ServiceActionReceived",HUMAN,|event|matches!(event,Event::ServiceActionReceived{chat_id,action:DecryptedMessageAction::AcceptKey(_),..} if *chat_id==id)).await?;
            let after = a.manager.status(id)?;
            require(
                after.key_fingerprint != before.key_fingerprint,
                "The official client accepted rekey but the local key did not change",
            )?;
            require(
                after.key_hash == before.key_hash,
                "The visual authentication hash changed during rekey",
            )?;
            let code = nonce()?;
            a.manager
                .send_message(id, &format!("rekey {code}"), None, None)
                .await?;
            println!(
                "REPLY in the official client's same secret chat with the new-key code: {code}"
            );
            received(a, id, &code).await?;
            require(
                !a.manager.status(id)?.has_previous_key,
                "The previous key was retained after a confirmed new-key reply without gaps",
            )?;
        }
    }
    require(
        !a.recorder.is_empty() && a.recorder.dropped() == 0,
        "The official-client observation was empty or exceeded its explicit bound",
    )?;
    println!(
        "Observed {} bounded live protocol events.",
        a.recorder.len()
    );
    require(
        a.recorder.of("DecryptFailed").is_empty(),
        "The official-client run contained a decryption or update-processing failure",
    )
}
async fn media_out(a: &Account, id: i32) -> TestResult {
    let jpeg = a.directory().join("interop-photo.jpg");
    let png = a.directory().join("interop-photo.png");
    image::RgbImage::from_pixel(1, 1, image::Rgb([32, 96, 160]))
        .save_with_format(&jpeg, image::ImageFormat::Jpeg)
        .map_err(|_| Refused("The real JPEG interop sample could not be generated"))?;
    image::RgbaImage::from_pixel(1, 1, image::Rgba([32, 96, 160, 255]))
        .save_with_format(&png, image::ImageFormat::Png)
        .map_err(|_| Refused("The real PNG interop sample could not be generated"))?;
    for (path, mime, label) in [
        (&jpeg, "image/jpeg", "photo-jpeg"),
        (&png, "image/png", "photo-png"),
    ] {
        a.manager
            .send_file(
                id,
                FileSource::path(path),
                SendFileOptions {
                    caption: label.into(),
                    mime_type: Some(mime.into()),
                    ..Default::default()
                },
            )
            .await?;
    }
    for (data, name, mime, label) in [
        (
            b"interop document body\n".to_vec(),
            "interop-note.txt",
            "text/plain",
            "text",
        ),
        (
            (0u8..=255).cycle().take(1024).collect(),
            "interop-blob.bin",
            "application/octet-stream",
            "binary",
        ),
    ] {
        a.manager
            .send_file(
                id,
                FileSource::bytes(data, name)?,
                SendFileOptions {
                    caption: label.into(),
                    mime_type: Some(mime.into()),
                    ..Default::default()
                },
            )
            .await?;
    }
    if let Some(path) = std::env::var_os("TSC_TEST_MEDIA_DIR") {
        let mut files = std::fs::read_dir(path)
            .map_err(|_| Refused("The explicitly supplied real-media directory is unavailable"))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Refused("The operator media directory could not be listed"))?;
        files.sort();
        for file in files.into_iter().filter(|file| file.is_file()) {
            a.manager
                .send_file(
                    id,
                    FileSource::path(file),
                    SendFileOptions {
                        caption: "operator-provided real sample".into(),
                        ..Default::default()
                    },
                )
                .await?;
        }
    } else {
        println!(
            "No real audio/video sample directory was supplied; this run covers JPEG, PNG, text and binary files."
        );
    }
    let confirmation = format!(
        "opened-{}",
        hex::encode(kiaro_secret_chat::crypto::random_bytes::<16>()?)
    );
    println!("OPEN every test file in the official client, then REPLY exactly: {confirmation}");
    let event=a.recorder.wait_for("MessageReceived",HUMAN,|event|matches!(event,Event::MessageReceived(message) if message.chat_id==id&&message.text.trim()==confirmation.as_str())).await?;
    require(
        matches!(event,Event::MessageReceived(message) if message.text.trim()==confirmation.as_str()),
        "The operator did not confirm that every actual file opened",
    )
}
pub async fn paired(case: Paired) -> TestResult {
    let _exclusive = super::LIVE_RUN.lock().await;
    let ca = Config::load("TSC_LIVE_SESSION_A")?;
    let cb = Config::load("TSC_LIVE_SESSION_B")?;
    require(
        ca.user_id != cb.user_id && ca.peer_user_id == cb.user_id && cb.peer_user_id == ca.user_id,
        "Pair tests require two distinct, mutually configured dedicated test accounts",
    )?;
    let mut a = Account::open(ca, false).await?;
    let mut b = match Account::open(cb, true).await {
        Ok(b) => b,
        Err(error) => {
            let _ = a.shutdown().await;
            return Err(error);
        }
    };
    let result =
        match tokio::time::timeout(Duration::from_secs(900), pair_case(&mut a, &mut b, case)).await
        {
            Ok(result) => result,
            Err(_) => {
                Err(Refused("The explicit real-account pair scenario exceeded its deadline").into())
            }
        };
    let cleanup_b = b.shutdown().await;
    let cleanup_a = a.shutdown().await;
    result.and(cleanup_b).and(cleanup_a)
}
async fn pair_case(a: &mut Account, b: &mut Account, case: Paired) -> TestResult {
    let id = a.ready("two-account pair tier").await?;
    b.recorder
        .wait_for("ChatReady", PAIR, |event| event.chat_id() == id)
        .await?;
    require(
        a.manager.status(id)?.key_fingerprint == b.manager.status(id)?.key_fingerprint,
        "The actual pair disagrees on the initial key fingerprint",
    )?;
    match case {
        Paired::Lifecycle => {
            require(
                a.manager.status(id)?.key_hash == b.manager.status(id)?.key_hash,
                "The actual pair disagrees on the visual authentication hash",
            )?;
            texts(a, b, id, "before-a").await?;
            texts(b, a, id, "before-b").await?;
            rekey_pair(a, b, id).await?;
            texts(a, b, id, "rekey-a").await?;
            texts(b, a, id, "rekey-b").await?;
            for (label, size) in [("small", 5 * 1024), ("multipart", 3 * 512 * 1024 + 11)] {
                file(
                    a,
                    b,
                    id,
                    &b.directory().join(format!("from-a-{label}.bin")),
                    size,
                )
                .await?;
                file(
                    b,
                    a,
                    id,
                    &a.directory().join(format!("from-b-{label}.bin")),
                    size,
                )
                .await?;
            }
            b.restart().await?;
            texts(a, b, id, "restart-a").await?;
            texts(b, a, id, "restart-b").await?;
        }
        Paired::CloseSync => {
            a.manager.close(id).await?;
            await_peer_closed(b, id, false).await?;
            require(
                b.recorder
                    .first("ChatClosedEvent", |event| event.chat_id() == id)
                    .is_some(),
                "The peer terminal update was not published",
            )?;
        }
        Paired::CloseSend => {
            a.manager.close(id).await?;
            require(
                b.manager
                    .send_message(id, "pair after peer closure", None, None)
                    .await
                    .is_err(),
                "The real server accepted a send into a peer-closed chat",
            )?;
            require(
                b.manager.status(id)?.state == ChatState::Closed,
                "The server rejection did not terminate the local chat",
            )?;
        }
        Paired::DeleteBoth => {
            require(
                a.manager.delete_secret_chat_both_sides(id).await?,
                "The real server did not confirm deletion for both sides",
            )?;
            require(
                !a.manager.list()?.iter().any(|chat| chat.id == id),
                "Deletion did not remove the local chat",
            )?;
            await_peer_closed(b, id, true).await?;
            require(b.recorder.first("ChatClosedEvent",|event|matches!(event,Event::ChatClosed{chat_id,history_deleted:true,..} if *chat_id==id)).is_some(),"The peer did not report history deletion")?;
        }
    }
    require(
        a.recorder.dropped() == 0 && b.recorder.dropped() == 0,
        "The pair observation exceeded its explicit bound",
    )?;
    require(
        a.recorder.of("DecryptFailed").is_empty() && b.recorder.of("DecryptFailed").is_empty(),
        "The actual pair observed a decryption or caller-stream failure",
    )
}
