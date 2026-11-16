//! Real participant pagination and membership regressions, inside the existing SDK case.
use super::*;

pub(super) fn unmapped_response(mode: u8) -> Option<Vec<u8>> {
    match mode {
        0 => None,
        1 => Some(tl::enums::Updates::TooLong.to_bytes()),
        2 => Some(
            tl::enums::Updates::UpdateShort(tl::types::UpdateShort {
                update: tl::types::UpdateDeleteMessages {
                    messages: vec![],
                    pts: 1,
                    pts_count: 0,
                }
                .into(),
                date: 123,
            })
            .to_bytes(),
        ),
        _ => panic!("unknown public fixture response"),
    }
}

pub(super) async fn accepted_sends(client: &Client, model: &Arc<Mutex<Model>>, peer: PeerRef) {
    for mode in [1, 2] {
        model.lock().unwrap().unsupported_send = mode;
        let before = model.lock().unwrap().calls.len();
        let accepted = client
            .send_message(peer, "accepted without response body")
            .await
            .unwrap();
        assert_eq!(accepted.id(), 0);
        assert_eq!(accepted.peer_id(), peer.id);
        assert_eq!(
            client
                .send_message(
                    peer,
                    InputMessage::new().photo_url("https://example.invalid/photo")
                )
                .await
                .unwrap()
                .id(),
            0
        );
        let media = InputMedia::new().media(tl::types::InputMediaPhoto {
            spoiler: false,
            id: tl::types::InputPhoto {
                id: 1,
                access_hash: 1,
                file_reference: vec![],
            }
            .into(),
            ttl_seconds: None,
            live_photo: false,
            video: None,
        });
        let album = client.send_album(peer, vec![media]).await.unwrap();
        assert_eq!(album.len(), 1);
        assert!(album[0].is_none());
        let forwarded = client.forward_messages(peer, &[1], peer).await.unwrap();
        assert_eq!(forwarded.len(), 1);
        assert!(forwarded[0].is_none());
        let self_peer = PeerRef::from(tl::enums::InputPeer::PeerSelf);
        let saved = client
            .send_message(self_peer, "accepted saved message")
            .await
            .unwrap();
        assert_eq!(saved.id(), 0);
        assert_eq!(saved.peer_id(), self_peer.id);
        assert_eq!(
            model.lock().unwrap().calls.len(),
            before + 5,
            "successful RPC was retried"
        );
    }
    model.lock().unwrap().unsupported_send = 0;
}

pub(super) fn channel_page(
    model: &mut Model,
    request: tl::functions::channels::GetParticipants,
) -> Vec<u8> {
    model.channel_offsets.push(request.offset);
    let (first, length, count) = if model.channel_full && request.offset == 0 {
        (1_000, 200, 201)
    } else if model.channel_full {
        assert_eq!(request.offset, 200);
        (1_200, 1, 201)
    } else {
        assert_eq!(request.offset, 0);
        (1_000, 1, 1)
    };
    let ids = first..first + length;
    tl::enums::channels::ChannelParticipants::Participants(
        tl::types::channels::ChannelParticipants {
            count,
            participants: ids
                .clone()
                .map(|user_id| {
                    tl::types::ChannelParticipantLeft {
                        peer: tl::types::PeerUser { user_id }.into(),
                    }
                    .into()
                })
                .collect(),
            chats: vec![],
            users: ids.map(|id| super::super::user(id, 0, Some(1))).collect(),
        },
    )
    .to_bytes()
}

pub(super) async fn participants(
    client: Client,
    session: Arc<PausingSession>,
    model: Arc<Mutex<Model>>,
    peer: PeerRef,
) {
    let channel = PeerRef {
        id: PeerId::channel_unchecked(10),
        auth: PeerAuth::from_hash(1),
    };
    for full in [false, true] {
        model.lock().unwrap().channel_full = full;
        model.lock().unwrap().channel_offsets.clear();
        let mut iter = client.iter_participants(channel);
        session.cache.store(true, Ordering::SeqCst);
        tokio::select! {
            result = iter.next() => panic!("participant page did not suspend at peer-cache: {result:?}"),
            _ = session.entered.notified() => {}
        }
        let first = iter
            .next()
            .await
            .unwrap()
            .expect("cancelled participant page must be retried");
        assert_eq!(
            first.peer.id(),
            PeerId::user_unchecked(1_000),
            "cancelled page cursor advanced"
        );
        let mut ids = vec![first.peer.id().bare_id_unchecked()];
        while let Some(participant) = iter.next().await.unwrap() {
            ids.push(participant.peer.id().bare_id_unchecked());
        }
        assert_eq!(
            ids,
            if full {
                (1_000..1_201).collect::<Vec<_>>()
            } else {
                vec![1_000]
            }
        );
        assert_eq!(
            model.lock().unwrap().channel_offsets,
            if full { vec![0, 0, 200] } else { vec![0, 0] }
        );
    }
    let chat = PeerRef {
        id: PeerId::chat_unchecked(10),
        auth: PeerAuth::default(),
    };
    for forbidden in [None, Some(false), Some(true)] {
        model.lock().unwrap().forbidden = forbidden;
        let permissions = client.get_permissions(chat, peer).await;
        assert_eq!(
            permissions.is_ok(),
            forbidden != Some(false),
            "forbidden self participant membership"
        );
        let nonmember = PeerRef {
            id: PeerId::user_unchecked(456),
            auth: PeerAuth::from_hash(1),
        };
        assert!(
            matches!(client.get_permissions(chat, nonmember).await, Err(InvocationError::Rpc(error)) if error.name=="USER_NOT_PARTICIPANT")
        );
    }
    model.lock().unwrap().forbidden = None;
}
