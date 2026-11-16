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
    file.write_all(b"{\"id\":2,\"text\":\"\xf0\x9f").unwrap();
    drop(file);
    assert_eq!(
        archive.read(7).unwrap(),
        vec![serde_json::json!({"id":1,"text":"kept"})],
        "a torn UTF-8 suffix must not hide earlier complete records"
    );
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

pub fn missing_alias_root_keeps_worker_generation() {
    use std::io::{Cursor, Read, Seek, SeekFrom};
    use std::sync::mpsc;
    struct Gated {
        source: Cursor<Vec<u8>>,
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }
    impl Read for Gated {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.release.recv().unwrap();
            }
            self.source.read(bytes)
        }
    }
    impl Seek for Gated {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.source.seek(position)
        }
    }
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("alias_component")).unwrap();
    let root = folder
        .path()
        .join("alias_component")
        .join("..")
        .join("missing_archive");
    let roots = vec![root];
    #[cfg(unix)]
    let roots = {
        let mut roots = roots;
        let base = folder.path().join("compound");
        let target = folder.path().join("physical-target");
        std::fs::create_dir(&base).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, base.join("alias")).unwrap();
        assert!(!base.join("absent").exists());
        roots.push(base.join("absent/../alias/archive"));
        roots
    };
    for root in roots {
        let (archive, _) = AutoSave::enabled(&root).unwrap();
        let worker = archive.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let copy = std::thread::spawn(move || {
            let mut source = kiaro_secret_chat::media::FileSource::stream(
                Box::new(Gated {
                    source: Cursor::new(b"private copy".to_vec()),
                    entered: Some(entered_tx),
                    release: release_rx,
                }),
                "kept.bin",
            )
            .unwrap()
            .open()
            .unwrap();
            worker.copy_source(7, 77, &mut source)
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        archive.delete(7).unwrap();
        release_tx.send(()).unwrap();
        let result = copy.join().unwrap();
        assert!(
            result.is_err() && !root.join("7").exists(),
            "creation must not change an archive worker's physical lock identity: {}",
            root.display()
        );
    }
}
