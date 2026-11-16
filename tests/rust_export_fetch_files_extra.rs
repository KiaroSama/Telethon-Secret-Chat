#![allow(clippy::unwrap_used)]
#[path = "rust_export_support/fetch_files.rs"]
mod support;
use support::*;
#[tokio::test]
async fn download_success_and_empty() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(vec![], vec![Ok(b"jpeg".to_vec()), Ok(vec![])]);
    let mut f = files(&c, d.path());
    let m = parsed(&tl_message(b"old", 7));
    let a = load(&mut f, &m).await;
    assert_eq!(fs::read(d.path().join(&a.relative_path)).unwrap(), b"jpeg");
    assert_eq!(f.files, 1);
    assert_eq!(
        *c.downloaded.lock().unwrap(),
        [(location(50, b"old", "y", false), 5)]
    );
    assert_eq!(
        f.file_cache.find(&a.location),
        Some(a.relative_path.as_str())
    );
    let mut other = m.clone();
    other.file_mut().unwrap().location = location(51, b"old", "y", false);
    let b = load(&mut f, &other).await;
    assert_ne!(b.relative_path, a.relative_path);
    assert_eq!(fs::read(d.path().join(b.relative_path)).unwrap(), b"");
}
#[tokio::test]
async fn unavailable_location() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(vec![], vec![Err(rpc("LOCATION_INVALID", 400))]);
    let mut f = files(&c, d.path());
    let a = load(&mut f, &parsed(&tl_message(b"old", 7))).await;
    assert_eq!(
        (a.relative_path, a.skip_reason),
        (String::new(), SkipReason::Unavailable)
    );
    assert_eq!(f.files, 0);
    assert_eq!(f.warnings, ["Export Error: File unavailable."]);
}
#[tokio::test]
async fn other_errors_propagate() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(vec![], vec![Err(rpc("FLOOD_WAIT_X", 420))]);
    let mut f = files(&c, d.path());
    let m = parsed(&tl_message(b"old", 7));
    let mut file = m.file();
    assert_eq!(
        f.process_file_load("", &mut file, &origin(), &m, 1, 5, &DateContext::utc())
            .await
            .unwrap_err(),
        rpc("FLOOD_WAIT_X", 420)
    );
}
#[tokio::test]
async fn reference_refresh_split_request() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(
        vec![Ok(messages(vec![tl_message(b"new", 7)]))],
        vec![Err(rpc("FILE_REFERENCE_EXPIRED", 400)), Ok(b"ok".to_vec())],
    );
    let mut f = files(&c, d.path());
    f.scope.takeout = Some(55);
    f.splits = vec![json!({"_":"MessageRange","min_id":1,"max_id":100})];
    let a = load(&mut f, &parsed(&tl_message(b"old", 7))).await;
    let calls = c.calls.lock().unwrap();
    assert_eq!(calls[0].0.range, f.splits.first().cloned());
    assert_eq!(calls[0].1["_"], "messages.GetMessages");
    assert_eq!(calls[0].1["id"][0]["id"], 7);
    assert_eq!(
        c.downloaded
            .lock()
            .unwrap()
            .iter()
            .map(|(l, _)| l.data["file_reference"].clone())
            .collect::<Vec<_>>(),
        [json!(b"old"), json!(b"new")]
    );
    assert_eq!(fs::read(d.path().join(a.relative_path)).unwrap(), b"ok");
}
#[tokio::test]
async fn refresh_missing_and_bounded_retries() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(
        vec![Ok(messages(vec![tl_message(b"new", 8)]))],
        vec![Err(rpc("FILE_REFERENCE_EXPIRED", 400))],
    );
    let mut f = files(&c, d.path());
    assert_eq!(
        load(&mut f, &parsed(&tl_message(b"old", 7)))
            .await
            .skip_reason,
        SkipReason::Unavailable
    );
    let c = Fake::scripted(
        vec![Ok(messages(vec![tl_message(b"new", 7)])); 3],
        vec![Err(rpc("FILE_REFERENCE_EXPIRED", 400)); 4],
    );
    let mut f = files(&c, d.path());
    assert_eq!(
        load(&mut f, &parsed(&tl_message(b"old", 7)))
            .await
            .skip_reason,
        SkipReason::Unavailable
    );
    assert_eq!(
        (
            c.downloaded.lock().unwrap().len(),
            c.calls.lock().unwrap().len()
        ),
        (4, 3)
    );
}
#[tokio::test]
async fn channel_reference_requests() {
    let d = tempfile::tempdir().unwrap();
    for (p, expected) in [
        (
            json!({"_":"InputPeerChannel","channel_id":9,"access_hash":4}),
            json!({"_":"InputChannel","channel_id":9,"access_hash":4}),
        ),
        (
            json!({"_":"InputPeerChannelFromMessage","peer":peer(),"msg_id":3,"channel_id":9}),
            json!({"_":"InputChannelFromMessage","peer":peer(),"msg_id":3,"channel_id":9}),
        ),
    ] {
        let c = Fake::scripted(vec![Ok(messages(vec![tl_message(b"new", 7)]))], vec![]);
        let mut f = files(&c, d.path());
        let a = f
            .refresh_reference(
                "",
                &FileOrigin {
                    peer: p,
                    message_id: 7,
                    ..Default::default()
                },
                &location(50, b"old", "y", false),
                &DateContext::utc(),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.data["file_reference"], json!(b"new"));
        let calls = c.calls.lock().unwrap();
        assert_eq!(calls[0].1["_"], "channels.GetMessages");
        assert_eq!(calls[0].1["channel"], expected);
    }
}
#[tokio::test]
async fn document_thumbnail_refresh() {
    let d = tempfile::tempdir().unwrap();
    let mut value = tl_message(b"old", 7);
    let document = |r: &[u8]| json!({"_":"Document","id":3,"access_hash":1,"file_reference":r,"date":1705053605,"mime_type":"application/pdf","size":100,"dc_id":2,"attributes":[],"thumbs":[{"_":"PhotoSize","type":"m","w":90,"h":90,"size":10}]});
    value["media"] = json!({"_":"MessageMediaDocument","document":document(b"old")});
    let stale = parsed(&value).thumb().file.location;
    assert_eq!(stale.data["thumb_size"], "m");
    value["media"]["document"] = document(b"new");
    let c = Fake::scripted(vec![Ok(messages(vec![value]))], vec![]);
    let mut f = files(&c, d.path());
    let a = f
        .refresh_reference("", &origin(), &stale, &DateContext::utc())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.data["file_reference"], json!(b"new"));
    assert_eq!(a.data["thumb_size"], "m");
}
#[tokio::test]
async fn reference_refresh_failures() {
    let d = tempfile::tempdir().unwrap();
    let c = Fake::scripted(vec![Err(rpc("CHANNEL_PRIVATE", 400))], vec![]);
    let mut f = files(&c, d.path());
    assert!(
        f.refresh_reference(
            "",
            &origin(),
            &location(50, b"old", "y", false),
            &DateContext::utc()
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        f.refresh_reference(
            "",
            &FileOrigin {
                peer: peer(),
                ..Default::default()
            },
            &location(50, b"old", "y", false),
            &DateContext::utc()
        )
        .await
        .is_err()
    );
    assert!(
        f.refresh_reference(
            "",
            &FileOrigin {
                peer: peer(),
                rich_message: true,
                ..Default::default()
            },
            &location(50, b"old", "y", false),
            &DateContext::utc()
        )
        .await
        .unwrap()
        .is_none()
    );
}
#[tokio::test]
async fn full_rich_reference_refresh() {
    let d = tempfile::tempdir().unwrap();
    let full = json!({"_":"Message","id":7,"peer_id":{"_":"PeerUser","user_id":100},"rich_message":{"_":"RichMessage","blocks":[],"photos":[photo(b"new")],"documents":[]}});
    let c = Fake::scripted(vec![Ok(messages(vec![full])), Ok(messages(vec![]))], vec![]);
    let mut f = files(&c, d.path());
    let mut origin = origin();
    origin.rich_message = true;
    let a = f
        .refresh_reference(
            "",
            &origin,
            &location(50, b"old", "y", false),
            &DateContext::utc(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(a.data["file_reference"], json!(b"new"));
    assert_eq!(
        c.calls.lock().unwrap()[0].1,
        json!({"_":"messages.GetRichMessage","peer":peer(),"id":7})
    );
    assert!(
        f.refresh_reference(
            "",
            &origin,
            &location(50, b"old", "y", false),
            &DateContext::utc()
        )
        .await
        .unwrap()
        .is_none()
    );
}
#[tokio::test]
async fn custom_emoji_reference_refresh() {
    let d = tempfile::tempdir().unwrap();
    for (answer, thumb, success) in [
        (json!([emoji(3, b"new")]), "", true),
        (json!([emoji(3, b"old"), emoji(3, b"old")]), "", false),
        (json!([emoji(4, b"old")]), "", false),
        (json!([emoji(3, b"new")]), "m", false),
    ] {
        let c = Fake::scripted(vec![Ok(answer)], vec![]);
        let mut f = files(&c, d.path());
        let result = f
            .refresh_reference(
                "",
                &FileOrigin {
                    custom_emoji_id: 3,
                    ..Default::default()
                },
                &location(3, b"old", thumb, true),
                &DateContext::utc(),
            )
            .await
            .unwrap();
        assert_eq!(result.is_some(), success);
        if let Some(v) = result {
            assert_eq!(v.data["file_reference"], json!(b"new"));
        }
        assert_eq!(c.calls.lock().unwrap()[0].1["document_id"], json!([3]));
    }
}

fn photo(reference: &[u8]) -> Value {
    json!({"_":"Photo","id":50,"access_hash":1,"file_reference":reference,"date":1705053605,"sizes":[{"_":"PhotoSize","type":"y","w":10,"h":10,"size":5}],"dc_id":2})
}

fn tl_message(reference: &[u8], id: i64) -> Value {
    json!({"_":"Message","id":id,"peer_id":{"_":"PeerUser","user_id":100},"date":1705053605,"message":"","media":{"_":"MessageMediaPhoto","photo":photo(reference)}})
}

fn messages(rows: Vec<Value>) -> Value {
    json!({"_":"Messages","messages":rows,"users":[],"chats":[]})
}

fn parsed(value: &Value) -> Message {
    parse_message(
        &mut ParseMediaContext {
            self_peer_id: 1,
            ..Default::default()
        },
        value,
        "",
        &DateContext::utc(),
    )
    .unwrap()
}

fn peer() -> Value {
    json!({"_":"InputPeerUser","user_id":100,"access_hash":5})
}

fn origin() -> FileOrigin {
    FileOrigin {
        split: 0,
        peer: peer(),
        message_id: 7,
        ..Default::default()
    }
}

async fn load(files: &mut MessageFiles<'_, Fake>, message: &Message) -> File {
    let mut f = message.file();
    let size = f.size;
    files
        .process_file_load(
            "",
            &mut f,
            &origin(),
            message,
            MediaSettings::PHOTO,
            size,
            &DateContext::utc(),
        )
        .await
        .unwrap();
    f
}
