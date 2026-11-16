#![allow(clippy::unwrap_used)]
#[path = "support_manager/logging.rs"]
mod logging;
#[path = "support_manager/mod.rs"]
mod support;
use kiaro_secret_chat::{
    manager::{Event, SendFileOptions},
    media::FileSource,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use support::*;
async fn saving() -> (Manager, Manager, Arc<Mutex<World>>, i32, tempfile::TempDir) {
    let (a, b, world, id) = established().await;
    let folder = tempfile::tempdir().unwrap();
    a.start_auto_save_secret_chats(folder.path()).await.unwrap();
    (a, b, world, id, folder)
}
async fn saved_files(a: &Manager, id: i32, count: usize) -> Vec<serde_json::Value> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let rows = a.read_saved_messages(id).unwrap();
            if rows.iter().filter(|r| r.get("file").is_some()).count() == count {
                return rows;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn messages_both_directions_include_ttl_and_service_records() {
    let (a, b, world, id, _folder) = saving().await;
    a.send_message(id, "from a", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    b.set_ttl(id, 5).await.unwrap();
    b.send_message(id, "self-destructing from b", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    let rows = a.read_saved_messages(id).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r["type"].as_str().unwrap(), r["out"].as_bool().unwrap()))
            .collect::<Vec<_>>(),
        [("message", true), ("service", false), ("message", false)]
    );
    assert_eq!(rows[0]["text"], "from a");
    assert_eq!(
        rows[1]["action"]["_"],
        "decryptedMessageActionSetMessageTTL"
    );
    assert_eq!(rows[2]["text"], "self-destructing from b");
    assert_eq!(rows[2]["ttl"], 5);
    assert!(rows.iter().all(|r| r["date"].as_u64().unwrap() > 0));
}
#[tokio::test]
async fn files_both_directions_are_saved_decrypted_without_keys() {
    let (a, b, world, id, _folder) = saving().await;
    let incoming = b"a photo from b".repeat(50);
    let outgoing = b"a document from a".repeat(70);
    b.send_file(
        id,
        FileSource::bytes(incoming.clone(), "in.jpg").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    a.send_file(
        id,
        FileSource::bytes(outgoing.clone(), "notes.txt").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    let rows = saved_files(&a, id, 2).await;
    for row in rows {
        let expected = if row["out"].as_bool().unwrap() {
            &outgoing
        } else {
            &incoming
        };
        assert_eq!(
            &std::fs::read(row["file"].as_str().unwrap()).unwrap(),
            expected
        );
        assert!(row["media"].get("key").is_none());
        assert!(row["media"].get("iv").is_none());
        assert!(row.get("reference").is_none());
    }
}
#[tokio::test]
async fn deleting_saved_conversation_removes_only_selected_folder() {
    let (a, _b, _world, id, folder) = saving().await;
    a.send_message(id, "x", None, None).await.unwrap();
    let unrelated = folder.path().join("unrelated.txt");
    std::fs::write(&unrelated, b"keep").unwrap();
    assert!(folder.path().join(id.to_string()).is_dir());
    a.delete_saved_messages(id).unwrap();
    assert!(!folder.path().join(id.to_string()).exists());
    assert!(a.read_saved_messages(id).unwrap().is_empty());
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep");
}
#[tokio::test]
async fn turning_archive_off_retains_prior_rows_and_stops_new_rows() {
    let (a, _b, _world, id, _folder) = saving().await;
    a.send_message(id, "saved", None, None).await.unwrap();
    a.stop_auto_save_secret_chats().await.unwrap();
    a.send_message(id, "not saved", None, None).await.unwrap();
    assert_eq!(
        a.read_saved_messages(id)
            .unwrap()
            .iter()
            .map(|r| r["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["saved"]
    );
}
#[tokio::test]
async fn archive_failure_never_breaks_delivery_or_send() {
    let capture = logging::Capture::default();
    let _subscriber = tracing::subscriber::set_default(capture.clone());
    let (a, b, world, id) = established().await;
    let folder = tempfile::tempdir().unwrap();
    let blocked = folder.path().join("file");
    std::fs::write(&blocked, []).unwrap();
    a.start_auto_save_secret_chats(&blocked).await.unwrap();
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&got);
    a.on("MessageReceived", "delivery", move |event| {
        if let Event::MessageReceived(m) = event {
            sink.lock().unwrap().push(m.text);
        }
        Ok(())
    })
    .unwrap();
    b.send_message(id, "still delivered", None, None)
        .await
        .unwrap();
    pump(&a, &b, &world).await;
    a.send_message(id, "still sent", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(*got.lock().unwrap(), ["still delivered"]);
    assert_eq!(b.read_history(id, 1).unwrap()[0].text, "still sent");
    let log = capture.0.lock().unwrap().join("\n");
    assert!(log.contains("Auto-save"));
    assert!(!log.contains("still"));
}
#[tokio::test]
async fn interrupted_file_download_is_resumed_on_next_start() {
    let (a, b, world, id, _folder) = saving().await;
    let gate = Arc::new(Gate::default());
    world.lock().unwrap().download_gate = Some(Arc::clone(&gate));
    let content = b"late bytes".repeat(20);
    b.send_file(
        id,
        FileSource::bytes(content.clone(), "late.bin").unwrap(),
        SendFileOptions::default(),
    )
    .await
    .unwrap();
    pump(&a, &b, &world).await;
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    a.stop().await.unwrap();
    world.lock().unwrap().download_gate = None;
    a.start().await.unwrap();
    let rows = saved_files(&a, id, 1).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        std::fs::read(rows[0]["file"].as_str().unwrap()).unwrap(),
        content
    );
}
#[tokio::test]
async fn saved_copy_survives_message_delete_and_both_sides_chat_delete() {
    let (a, b, world, id, _folder) = saving().await;
    let random_id = b.send_message(id, "keep me", None, None).await.unwrap();
    pump(&a, &b, &world).await;
    b.delete_messages(id, vec![random_id]).await.unwrap();
    pump(&a, &b, &world).await;
    a.delete_secret_chat_both_sides(id).await.unwrap();
    pump(&a, &b, &world).await;
    assert_eq!(
        a.read_saved_messages(id)
            .unwrap()
            .iter()
            .filter(|r| r["type"] == "message")
            .map(|r| r["text"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["keep me"]
    );
}
