use grammers_tl_types as tl;
use kiaro_secret_chat::{autosave::AutoSave, files::FileKey, media::MediaReference, types};
use std::{
    collections::BTreeSet,
    sync::{Arc, Barrier},
};
pub fn concurrent_archive_transactions() {
    let dir = tempfile::tempdir().unwrap();
    let (archive, _) = AutoSave::enabled(dir.path()).unwrap();
    let key = FileKey::new(&[1; 32], &[2; 32]).unwrap();
    let reference = MediaReference::new(
        7,
        &tl::types::EncryptedFile {
            id: 77,
            access_hash: 88,
            size: 16,
            dc_id: 2,
            key_fingerprint: key.fingerprint(),
        },
        &types::DecryptedMessageMediaDocument {
            thumb: vec![],
            thumb_w: 0,
            thumb_h: 0,
            mime_type: "application/octet-stream".into(),
            size: 1,
            key: key.key().to_vec(),
            iv: key.iv().to_vec(),
            attributes: vec![],
            caption: String::new(),
        }
        .into(),
    )
    .unwrap();
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|threads| {
        for lane in 0..8 {
            let archive = archive.clone();
            let reference = reference.clone();
            let barrier = Arc::clone(&barrier);
            threads.spawn(move||{barrier.wait();for row in 0..12 {let id=lane*12+row;archive.append(7,&serde_json::json!({"type":"message","id":id,"text":"public concurrent archive fixture ".repeat(128)})).unwrap();archive.owe(7,id,&reference,"fixture.bin").unwrap();}});
        }
    });
    assert_eq!(
        archive.read(7).unwrap().len(),
        96,
        "concurrent record writes must stay distinct JSON lines"
    );
    assert_eq!(
        archive.pending(7).unwrap().len(),
        96,
        "concurrent registration must retain every obligation"
    );
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|threads| {
        for lane in 0..8 {
            let archive = archive.clone();
            let reference = reference.clone();
            let barrier = Arc::clone(&barrier);
            let path = dir.path().join("public-completed-fixture.bin");
            threads.spawn(move || {
                barrier.wait();
                for row in 0..12 {
                    if lane < 4 {
                        archive
                            .completed_file(7, 2 * (lane * 12 + row), &path)
                            .unwrap();
                    } else {
                        archive
                            .owe(7, 96 + (lane - 4) * 12 + row, &reference, "new.bin")
                            .unwrap();
                    }
                }
            });
        }
    });
    let actual = archive
        .pending(7)
        .unwrap()
        .into_keys()
        .map(|key| key.parse::<i64>().unwrap())
        .collect::<BTreeSet<_>>();
    let expected = (0..96)
        .filter(|id| id % 2 == 1)
        .chain(96..144)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "completion and registration must not erase or resurrect each other's ledger rows"
    );
    let rows = archive.read(7).unwrap();
    assert_eq!(rows.len(), 96);
    assert_eq!(
        rows.iter().filter(|row| row.get("file").is_some()).count(),
        48
    );
}

pub fn incomplete_tail_does_not_eat_next_record() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let (archive, _) = AutoSave::enabled(dir.path()).unwrap();
    archive
        .append(7, &serde_json::json!({"id":1,"text":"kept"}))
        .unwrap();
    let path = dir.path().join("7/messages.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"{\"id\":2").unwrap();
    drop(file);
    archive
        .append(
            7,
            &serde_json::json!({"id":3,"text":"successful later record"}),
        )
        .unwrap();
    assert_eq!(
        archive
            .read(7)
            .unwrap()
            .iter()
            .map(|row| row["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 3],
        "an interrupted old tail cannot swallow a later successful append"
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"{\"id\":4}").unwrap();
    drop(file);
    archive.append(7, &serde_json::json!({"id":5})).unwrap();
    assert_eq!(
        archive
            .read(7)
            .unwrap()
            .iter()
            .map(|row| row["id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 3, 4, 5],
        "a valid complete final JSON value must survive newline recovery"
    );
}
