#![allow(clippy::unwrap_used)]
use super::support;
use kiaro_secret_chat::{
    error::Error,
    manager::{Event, MessageReceived, SendFileOptions},
    media::{FileSource, file_parts},
};
use std::sync::{Arc, Mutex};
fn collected(manager: &support::Manager) -> Arc<Mutex<Vec<MessageReceived>>> {
    let rows = Arc::new(Mutex::new(Vec::new()));
    let sink = rows.clone();
    manager
        .on("MessageReceived", "primitive_file", move |e| {
            if let Event::MessageReceived(m) = e {
                sink.lock().unwrap().push(*m);
            }
            Ok(())
        })
        .unwrap();
    rows
}
pub async fn run(name: &str) {
    let (a, b, world, id) = support::established().await;
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("f.bin");
    let data = match name {
        "test_a_multi_part_file_streams_both_ways_without_a_memory_copy" => {
            (0..3 * 512 * 1024 + 11).map(|i| (i % 251) as u8).collect()
        }
        "test_the_server_only_ever_sees_ciphertext" => b"RECOGNISABLE".repeat(100),
        _ => vec![b'x'; 5000],
    };
    std::fs::write(&source, &data).unwrap();
    let got = collected(&b);
    match name {
        "test_sending_an_unreadable_file_refuses" => {
            let result = a
                .send_file(
                    id,
                    FileSource::path(directory.path().join("missing.bin")),
                    SendFileOptions::default(),
                )
                .await;
            assert!(result.is_err());
            assert_eq!(world.lock().unwrap().uploads, 0);
        }
        "test_sending_a_file_refuses_on_a_chat_that_is_not_ready" => {
            let pending = a.create(support::peer(2000)).await.unwrap().id;
            let e = a
                .send_file(
                    pending,
                    FileSource::path(&source),
                    SendFileOptions::default(),
                )
                .await
                .unwrap_err();
            assert!(matches!(e, Error::ChatNotReady { .. }));
            assert_eq!(world.lock().unwrap().uploads, 0);
        }
        "test_a_message_arrives_while_a_file_is_uploading"
        | "test_a_text_sent_during_an_upload_lands_after_the_file" => {
            let gate = Arc::new(support::Gate::default());
            world.lock().unwrap().upload_gate = Some(gate.clone());
            let send_a = a.clone();
            let upload = tokio::spawn(async move {
                send_a
                    .send_file(id, FileSource::path(&source), SendFileOptions::default())
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified())
                .await
                .unwrap();
            if name == "test_a_message_arrives_while_a_file_is_uploading" {
                let inbound = collected(&a);
                b.send_message(id, "while you upload", None, None)
                    .await
                    .unwrap();
                support::pump(&a, &b, &world).await;
                assert_eq!(
                    inbound.lock().unwrap().last().unwrap().text,
                    "while you upload"
                );
                gate.release.notify_one();
                upload.await.unwrap().unwrap();
            } else {
                let text_a = a.clone();
                let text = tokio::spawn(async move {
                    text_a.send_message(id, "after the file", None, None).await
                });
                tokio::task::yield_now().await;
                assert!(!text.is_finished());
                gate.release.notify_one();
                upload.await.unwrap().unwrap();
                text.await.unwrap().unwrap();
                support::pump(&a, &b, &world).await;
                let messages = got.lock().unwrap();
                assert_eq!(messages.len(), 2);
                assert!(messages[0].media.is_some());
                assert_eq!(messages[1].text, "after the file");
            }
        }
        _ => {
            a.send_file(id, FileSource::path(&source), SendFileOptions::default())
                .await
                .unwrap();
            support::pump(&a, &b, &world).await;
            let event = got.lock().unwrap().last().unwrap().clone();
            let reference = event.media_reference.as_ref().unwrap();
            match name {
                "test_a_file_crosses_the_chat_and_opens_on_the_other_side" => {
                    let target = directory.path().join("received.bin");
                    let saved = b.save_file(reference, &target).await.unwrap();
                    assert_eq!(std::fs::read(saved).unwrap(), data);
                }
                "test_the_server_only_ever_sees_ciphertext" => {
                    let w = world.lock().unwrap();
                    assert_eq!(w.uploads, 1);
                    for file in w.files.values() {
                        assert!(!file.windows(12).any(|w| w == b"RECOGNISABLE"));
                    }
                    let outer = w.sent.last().unwrap();
                    assert!(
                        !outer
                            .outgoing
                            .frame
                            .windows(12)
                            .any(|w| w == b"RECOGNISABLE")
                    );
                }
                "test_the_key_travels_inside_and_the_fingerprint_outside" => {
                    let media = event.media.as_ref().unwrap();
                    let parts = file_parts(media).unwrap();
                    assert_eq!((parts.key.key().len(), parts.key.iv().len()), (32, 32));
                    assert_eq!(
                        event.file.as_ref().unwrap().key_fingerprint,
                        parts.key.fingerprint()
                    );
                    let ciphertext = world
                        .lock()
                        .unwrap()
                        .sent
                        .last()
                        .unwrap()
                        .outgoing
                        .frame
                        .clone();
                    assert!(!ciphertext.windows(32).any(|w| w == parts.key.key()));
                    assert!(!ciphertext.windows(32).any(|w| w == parts.key.iv()));
                }
                "test_saving_a_file_whose_fingerprint_is_wrong_writes_nothing" => {
                    let mut reference = reference.clone();
                    reference.key_fingerprint ^= 255;
                    let target = directory.path().join("must-not-appear.bin");
                    let before = world.lock().unwrap().downloads;
                    assert!(b.save_file(&reference, &target).await.is_err());
                    assert!(!target.exists());
                    assert_eq!(world.lock().unwrap().downloads, before);
                }
                "test_a_multi_part_file_streams_both_ways_without_a_memory_copy" => {
                    let target = directory.path().join("out/big.bin");
                    b.save_file(reference, &target).await.unwrap();
                    assert_eq!(std::fs::read(&target).unwrap(), data);
                    assert_eq!(
                        std::fs::read_dir(target.parent().unwrap()).unwrap().count(),
                        1
                    );
                    let ast = syn::parse_file(include_str!("../support_manager/mod.rs")).unwrap();
                    let methods = ast.items.iter().filter_map(|x| {
                        if let syn::Item::Impl(i) = x {
                            Some(i)
                        } else {
                            None
                        }
                    });
                    let mut found = false;
                    for implementation in methods {
                        for item in &implementation.items {
                            if let syn::ImplItem::Fn(f) = item
                                && f.sig.ident == "download"
                            {
                                found = true;
                                struct Sinks(bool);
                                impl<'a> syn::visit::Visit<'a> for Sinks {
                                    fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
                                        if let syn::Expr::Path(p) = &*c.func
                                            && p.path
                                                .segments
                                                .iter()
                                                .any(|s| s.ident == "save_file")
                                        {
                                            self.0 = true;
                                        }
                                        syn::visit::visit_expr_call(self, c);
                                    }
                                }
                                let mut sink = Sinks(false);
                                syn::visit::Visit::visit_block(&mut sink, &f.block);
                                assert!(
                                    sink.0,
                                    "download must use actual disk-streaming save_file"
                                );
                            }
                        }
                    }
                    assert!(found);
                }
                _ => panic!("unknown file transport scenario {name}"),
            }
        }
    }
}
