//! Actual SDK page cancellation, message and participant-builder regressions.
use kiaro_secret_chat::{
    grammers_client::{
        Client,
        media::InputMedia,
        message::{InputMessage, InputRichMessage},
        peer::PeerMap,
        update::Update,
    },
    grammers_mtsender::{InvocationError, SenderPool},
    grammers_session::{
        BoxFuture, Session, SessionData,
        storages::MemorySession,
        types::{DcOption, PeerAuth, PeerId, PeerInfo, PeerRef, UpdateState, UpdatesState},
        updates::State,
    },
    grammers_tl_types::{self as tl, Identifiable, Serializable},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
#[path = "review_pages_ea3.rs"]
mod ea3;

struct PausingSession {
    inner: MemorySession,
    cache: AtomicBool,
    peer: AtomicBool,
    fail_peer: AtomicBool,
    entered: Notify,
}
impl Session for PausingSession {
    type Error = std::io::Error;
    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.inner.home_dc_id().unwrap())
    }
    fn dc_option(&self, dc: i32) -> Result<Option<DcOption>, Self::Error> {
        Ok(self.inner.dc_option(dc).unwrap())
    }
    fn set_home_dc_id(&self, dc: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.inner.set_home_dc_id(dc).await.unwrap();
            Ok(())
        })
    }
    fn set_dc_option(&self, dc: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        let dc = dc.clone();
        Box::pin(async move {
            self.inner.set_dc_option(&dc).await.unwrap();
            Ok(())
        })
    }
    fn peer(&self, id: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move { Ok(self.inner.peer(id).await.unwrap()) })
    }
    fn peer_ref(&self, id: PeerId) -> BoxFuture<'_, Result<Option<PeerRef>, Self::Error>> {
        Box::pin(async move {
            if self.peer.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            if self.fail_peer.swap(false, Ordering::SeqCst) {
                return Err(std::io::Error::other("public fixture peer lookup failure"));
            }
            Ok(self.inner.peer_ref(id).await.unwrap())
        })
    }
    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.inner.cache_peer(peer).await.unwrap();
            Ok(())
        })
    }
    fn cache_peers(&self, peers: Vec<PeerInfo>) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            if self.cache.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            self.inner.cache_peers(peers).await.unwrap();
            Ok(())
        })
    }
    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.inner.updates_state().await.unwrap()) })
    }
    fn set_update_state(&self, state: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.inner.set_update_state(state).await.unwrap();
            Ok(())
        })
    }
}
#[derive(Default)]
struct Model {
    calls: Vec<(i32, u32)>,
    history: usize,
    global: Vec<(i32, i32)>,
    missing_forward: bool,
    unsupported_send: u8,
    channel_full: bool,
    channel_offsets: Vec<i32>,
    forbidden: Option<bool>,
    role: u8,
    admin_flags: Vec<(bool, String)>,
    banned_flags: Vec<(bool, i32)>,
    expected_reply: Option<i32>,
    rich_edits: usize,
}
fn messages(list: Vec<tl::enums::Message>) -> Vec<u8> {
    tl::enums::messages::Messages::Messages(tl::types::messages::Messages {
        messages: list,
        topics: vec![],
        chats: vec![],
        users: vec![super::user(123, 0, Some(1))],
    })
    .to_bytes()
}
fn combined(randoms: &[i64]) -> Vec<u8> {
    let updates = randoms
        .iter()
        .enumerate()
        .flat_map(|(index, random)| {
            let id = 100 + index as i32;
            [
                tl::types::UpdateMessageId {
                    id,
                    random_id: *random,
                }
                .into(),
                tl::types::UpdateNewMessage {
                    message: super::service(id, 123),
                    pts: 1,
                    pts_count: 1,
                }
                .into(),
            ]
        })
        .collect();
    tl::enums::Updates::Combined(tl::types::UpdatesCombined {
        updates,
        users: vec![super::user(123, 0, Some(1))],
        chats: vec![],
        date: 123,
        seq_start: 1,
        seq: 1,
    })
    .to_bytes()
}
fn raw_message() -> tl::enums::Message {
    let mut bytes = tl::types::Message::CONSTRUCTOR_ID.to_le_bytes().to_vec();
    bytes.extend([0; 8]);
    bytes.extend(1i32.to_le_bytes());
    bytes.extend(tl::enums::Peer::User(tl::types::PeerUser { user_id: 123 }).to_bytes());
    bytes.extend(123i32.to_le_bytes());
    bytes.extend("<b>x</b> **x**".to_owned().to_bytes());
    super::super::decode(&bytes)
}
fn service_reply() -> tl::enums::Message {
    let tl::enums::Message::Service(mut service) = super::service(2, 123) else {
        unreachable!()
    };
    let mut header = tl::types::MessageReplyHeader::CONSTRUCTOR_ID
        .to_le_bytes()
        .to_vec();
    header.extend((1u32 << 4).to_le_bytes());
    header.extend(42i32.to_le_bytes());
    service.reply_to = Some(super::super::decode(&header));
    service.into()
}
fn rights<T: tl::Deserializable>(id: u32, flags: u32, tail: &[u8]) -> T {
    let mut bytes = id.to_le_bytes().to_vec();
    bytes.extend(flags.to_le_bytes());
    bytes.extend(tail);
    super::super::decode(&bytes)
}
impl Model {
    fn invoke(&mut self, dc: i32, body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let id = u32::from_le_bytes(body[..4].try_into().unwrap());
        self.calls.push((dc, id));
        Ok(match id {
            id if id == tl::functions::users::GetUsers::CONSTRUCTOR_ID => {
                vec![super::user(123, 0, Some(1))].to_bytes()
            }
            id if id == tl::functions::messages::GetMessages::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::GetMessages =
                    super::super::decode(&body[4..]);
                let reply = request.id.iter().any(|id| {
                    matches!(
                        id,
                        tl::enums::InputMessage::ReplyTo(_)
                            | tl::enums::InputMessage::Id(tl::types::InputMessageId { id: 42 })
                    )
                });
                if reply {
                    messages(vec![super::service(42, 123)])
                } else {
                    messages(vec![raw_message(), service_reply()])
                }
            }
            id if id == tl::functions::messages::GetHistory::CONSTRUCTOR_ID => {
                self.history += 1;
                messages(vec![super::service(1, 123)])
            }
            id if id == tl::functions::messages::SearchGlobal::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::SearchGlobal =
                    super::super::decode(&body[4..]);
                self.global.push((request.offset_rate, request.offset_id));
                if request.offset_id == 0 {
                    tl::enums::messages::Messages::Slice(tl::types::messages::MessagesSlice {
                        inexact: false,
                        count: 2,
                        next_rate: Some(9),
                        offset_id_offset: None,
                        search_flood: None,
                        messages: vec![super::service(1, 123)],
                        topics: vec![],
                        chats: vec![],
                        users: vec![super::user(123, 1 << 20, None)],
                    })
                    .to_bytes()
                } else {
                    assert_eq!((request.offset_rate, request.offset_id), (9, 1));
                    assert!(
                        matches!(request.offset_peer,tl::enums::InputPeer::User(peer) if peer.user_id==123)
                    );
                    messages(vec![super::service(2, 123)])
                }
            }
            id if id == tl::functions::messages::Search::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::Search = super::super::decode(&body[4..]);
                assert!(matches!(
                    request.filter,
                    tl::enums::MessagesFilter::InputMessagesFilterPinned
                ));
                let peer = PeerRef::from(request.peer).id.bare_id_unchecked();
                messages(vec![super::service(7, peer)])
            }
            id if id == tl::functions::messages::SendMessage::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::SendMessage =
                    super::super::decode(&body[4..]);
                if let Some(response) = ea3::unmapped_response(self.unsupported_send) {
                    response
                } else {
                    combined(&[request.random_id])
                }
            }
            id if id == tl::functions::messages::SendMedia::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::SendMedia = super::super::decode(&body[4..]);
                ea3::unmapped_response(self.unsupported_send)
                    .unwrap_or_else(|| combined(&[request.random_id]))
            }
            id if id == tl::functions::messages::SendMultiMedia::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::SendMultiMedia =
                    super::super::decode(&body[4..]);
                if let Some(expected) = self.expected_reply {
                    assert!(
                        matches!(request.reply_to,Some(tl::enums::InputReplyTo::Message(reply)) if reply.reply_to_msg_id==expected)
                    );
                }
                if let Some(response) = ea3::unmapped_response(self.unsupported_send) {
                    return Ok(response);
                }
                combined(
                    &request
                        .multi_media
                        .iter()
                        .map(|media| {
                            let tl::enums::InputSingleMedia::Media(media) = media;
                            media.random_id
                        })
                        .collect::<Vec<_>>(),
                )
            }
            id if id == tl::functions::messages::ForwardMessages::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::ForwardMessages =
                    super::super::decode(&body[4..]);
                if let Some(response) = ea3::unmapped_response(self.unsupported_send) {
                    return Ok(response);
                }
                combined(if self.missing_forward {
                    &[]
                } else {
                    &request.random_id
                })
            }
            id if id == tl::functions::messages::EditMessage::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::EditMessage =
                    super::super::decode(&body[4..]);
                assert!(
                    matches!(request.rich_message,Some(tl::enums::InputRichMessage::Html(html)) if html.html=="<b>rich</b>")
                );
                self.rich_edits += 1;
                combined(&[])
            }
            id if id == tl::functions::auth::ExportAuthorization::CONSTRUCTOR_ID => {
                let mut bytes = tl::types::auth::ExportedAuthorization::CONSTRUCTOR_ID
                    .to_le_bytes()
                    .to_vec();
                bytes.extend(
                    tl::types::auth::ExportedAuthorization {
                        id: 1,
                        bytes: vec![1],
                    }
                    .to_bytes(),
                );
                bytes
            }
            id if id == tl::functions::auth::ImportAuthorization::CONSTRUCTOR_ID => {
                let mut bytes = tl::types::auth::Authorization::CONSTRUCTOR_ID
                    .to_le_bytes()
                    .to_vec();
                bytes.extend(0u32.to_le_bytes());
                bytes.extend(super::user(123, 0, Some(1)).to_bytes());
                bytes
            }
            id if id == tl::functions::messages::SetBotCallbackAnswer::CONSTRUCTOR_ID => {
                true.to_bytes()
            }
            id if id == tl::functions::messages::EditInlineBotMessage::CONSTRUCTOR_ID => {
                assert_eq!(dc, 4, "inline message must route to its own DC");
                true.to_bytes()
            }
            id if id == tl::functions::messages::GetFullChat::CONSTRUCTOR_ID => {
                let participant = tl::types::ChatParticipantAdmin {
                    user_id: 123,
                    inviter_id: 123,
                    date: 0,
                    rank: None,
                };
                let participants = match self.forbidden {
                    Some(own) => tl::types::ChatParticipantsForbidden {
                        chat_id: 10,
                        self_participant: own.then(|| participant.clone().into()),
                    }
                    .into(),
                    None => tl::types::ChatParticipants {
                        chat_id: 10,
                        participants: if self.role == 0 {
                            vec![participant.into()]
                        } else {
                            vec![]
                        },
                        version: 1,
                    }
                    .into(),
                };
                let mut full = tl::types::ChatFull::CONSTRUCTOR_ID.to_le_bytes().to_vec();
                full.extend(0u32.to_le_bytes());
                full.extend(10i64.to_le_bytes());
                full.extend(String::new().to_bytes());
                full.extend(tl::enums::ChatParticipants::to_bytes(&participants));
                full.extend(tl::types::PeerNotifySettings::CONSTRUCTOR_ID.to_le_bytes());
                full.extend(0u32.to_le_bytes());
                tl::enums::messages::ChatFull::Full(tl::types::messages::ChatFull {
                    full_chat: super::super::decode(&full),
                    chats: vec![],
                    users: vec![super::user(123, 0, Some(1))],
                })
                .to_bytes()
            }
            id if id == tl::functions::channels::GetParticipants::CONSTRUCTOR_ID => {
                let request = super::super::decode(&body[4..]);
                ea3::channel_page(self, request)
            }
            id if id == tl::functions::channels::GetParticipant::CONSTRUCTOR_ID => {
                let participant = match self.role {
                    0 => tl::types::ChannelParticipantAdmin {
                        can_edit: false,
                        is_self: false,
                        user_id: 123,
                        inviter_id: None,
                        promoted_by: 123,
                        date: 0,
                        admin_rights: rights(tl::types::ChatAdminRights::CONSTRUCTOR_ID, 1, &[]),
                        rank: Some("old rank".into()),
                    }
                    .into(),
                    2 => tl::types::ChannelParticipantBanned {
                        left: false,
                        peer: tl::types::PeerUser { user_id: 123 }.into(),
                        kicked_by: 123,
                        date: 0,
                        banned_rights: rights(
                            tl::types::ChatBannedRights::CONSTRUCTOR_ID,
                            2,
                            &0i32.to_le_bytes(),
                        ),
                        rank: None,
                    }
                    .into(),
                    _ => tl::types::ChannelParticipant {
                        user_id: 123,
                        date: 0,
                        subscription_until_date: None,
                        rank: None,
                    }
                    .into(),
                };
                tl::enums::channels::ChannelParticipant::Participant(
                    tl::types::channels::ChannelParticipant {
                        participant,
                        chats: vec![],
                        users: vec![super::user(123, 0, Some(1))],
                    },
                )
                .to_bytes()
            }
            id if id == tl::functions::channels::EditAdmin::CONSTRUCTOR_ID => {
                let request: tl::functions::channels::EditAdmin = super::super::decode(&body[4..]);
                let tl::enums::ChatAdminRights::Rights(rights) = request.admin_rights;
                self.admin_flags
                    .push((rights.change_info, request.rank.unwrap_or_default()));
                combined(&[])
            }
            id if id == tl::functions::messages::EditChatAdmin::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::EditChatAdmin =
                    super::super::decode(&body[4..]);
                self.admin_flags.push((request.is_admin, String::new()));
                true.to_bytes()
            }
            id if id == tl::functions::channels::EditBanned::CONSTRUCTOR_ID => {
                let request: tl::functions::channels::EditBanned = super::super::decode(&body[4..]);
                let tl::enums::ChatBannedRights::Rights(rights) = request.banned_rights;
                self.banned_flags
                    .push((rights.send_messages, rights.until_date));
                combined(&[])
            }
            _ => panic!("unexpected review-model RPC {id:x}"),
        })
    }
}
pub async fn run(state: State, peers: PeerMap) {
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some([7; 256]);
    data.dc_options.get_mut(&4).unwrap().auth_key = Some([8; 256]);
    let session = Arc::new(PausingSession {
        inner: MemorySession::from(data),
        cache: AtomicBool::new(false),
        peer: AtomicBool::new(false),
        fail_peer: AtomicBool::new(false),
        entered: Notify::new(),
    });
    session
        .cache_peer(PeerInfo::User {
            id: 123,
            auth: Some(PeerAuth::from_hash(1)),
            bot: None,
            is_self: None,
        })
        .await
        .unwrap();
    let pool = SenderPool::new(session.clone(), 123);
    let handle = pool.handle.clone();
    let client = Client::new(handle.clone());
    let model = Arc::new(Mutex::new(Model::default()));
    let remote = model.clone();
    let runner = tokio::spawn(pool.runner.run_offline_fixture(move |dc, body| {
        let remote = remote.clone();
        async move { remote.lock().unwrap().invoke(dc, body) }
    }));
    let peer = PeerRef {
        id: PeerId::user_unchecked(123),
        auth: PeerAuth::from_hash(1),
    };
    Box::pin(pagination_checks(
        client.clone(),
        session.clone(),
        model.clone(),
        peer,
    ))
    .await;
    Box::pin(ea3::participants(
        client.clone(),
        session,
        model.clone(),
        peer,
    ))
    .await;
    Box::pin(message_checks(
        client.clone(),
        model.clone(),
        peer,
        state,
        peers,
    ))
    .await;
    Box::pin(participant_checks(client, model, peer)).await;
    handle.quit();
    runner.await.unwrap();
}

async fn pagination_checks(
    client: Client,
    session: Arc<PausingSession>,
    model: Arc<Mutex<Model>>,
    peer: PeerRef,
) {
    let mut iter = client.iter_messages(peer);
    session.cache.store(true, Ordering::SeqCst);
    tokio::select! { result=iter.next()=>panic!("final page did not suspend: {result:?}"), _=session.entered.notified()=>{} }
    assert_eq!(iter.next().await.unwrap().unwrap().id(), 1);
    assert!(iter.next().await.unwrap().is_none());
    assert_eq!(model.lock().unwrap().history, 2);
    for fail in [false, true] {
        let mut iter = client.search_all_messages();
        if fail {
            session.fail_peer.store(true, Ordering::SeqCst);
            assert!(iter.next().await.is_err());
        } else {
            session.peer.store(true, Ordering::SeqCst);
            tokio::select! {result=iter.next()=>panic!("cursor did not suspend: {result:?}"),_=session.entered.notified()=>{}}
        }
        assert_eq!(iter.next().await.unwrap().unwrap().id(), 1);
        assert_eq!(iter.next().await.unwrap().unwrap().id(), 2);
        assert!(iter.next().await.unwrap().is_none());
    }
    assert_eq!(
        model.lock().unwrap().global,
        [(0, 0), (0, 0), (9, 1), (0, 0), (0, 0), (9, 1)]
    );
    assert!(
        client
            .get_messages_by_id(tl::enums::InputPeer::PeerSelf, &[1])
            .await
            .unwrap()[0]
            .is_some()
    );
    assert!(
        client
            .get_messages_by_id(
                PeerRef {
                    id: PeerId::user_unchecked(456),
                    auth: PeerAuth::from_hash(1)
                },
                &[1]
            )
            .await
            .unwrap()[0]
            .is_none()
    );
    for id in [123, 456] {
        let peer = PeerRef {
            id: PeerId::user_unchecked(id),
            auth: PeerAuth::from_hash(1),
        };
        assert_eq!(
            client
                .get_pinned_message(peer)
                .await
                .unwrap()
                .unwrap()
                .peer_id(),
            peer.id
        );
    }
}

async fn message_checks(
    client: Client,
    model: Arc<Mutex<Model>>,
    peer: PeerRef,
    state: State,
    peers: PeerMap,
) {
    let message = client.get_messages_by_id(peer, &[1, 2]).await.unwrap();
    let normal = message[0].as_ref().unwrap();
    let service = message[1].as_ref().unwrap();
    assert_eq!(normal.html_text(), "&lt;b&gt;x&lt;/b&gt; **x**");
    assert_eq!(
        kiaro_secret_chat::grammers_client::parsers::parse_markdown_message(
            &normal.markdown_text()
        )
        .0,
        normal.text()
    );
    assert_eq!(service.reply_to_message_id(), Some(42));
    assert_eq!(service.get_reply().await.unwrap().unwrap().id(), 42);
    let before = model.lock().unwrap().calls.len();
    assert!(normal.reply_album(vec![]).await.is_err());
    assert_eq!(model.lock().unwrap().calls.len(), before);
    let media = || {
        InputMedia::new().media(tl::types::InputMediaPhoto {
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
        })
    };
    model.lock().unwrap().expected_reply = Some(1);
    assert_eq!(
        normal
            .reply_album(vec![InputMedia::new(), media()])
            .await
            .unwrap()
            .len(),
        1
    );
    model.lock().unwrap().expected_reply = None;
    assert_eq!(
        client.send_album(peer, vec![media()]).await.unwrap()[0]
            .as_ref()
            .unwrap()
            .id(),
        100
    );
    assert_eq!(
        client
            .send_message(peer, "combined response")
            .await
            .unwrap()
            .id(),
        100
    );
    assert_eq!(normal.forward_to(peer).await.unwrap().id(), 100);
    model.lock().unwrap().missing_forward = true;
    assert!(normal.forward_to(peer).await.is_err());
    model.lock().unwrap().missing_forward = false;
    Box::pin(ea3::accepted_sends(&client, &model, peer)).await;
    client
        .edit_message(
            peer,
            1,
            InputMessage::new().rich_message(InputRichMessage::html("<b>rich</b>")),
        )
        .await
        .unwrap();
    assert_eq!(model.lock().unwrap().rich_edits, 1);
    let inline = tl::types::UpdateInlineBotCallbackQuery {
        query_id: 2,
        user_id: 123,
        msg_id: tl::types::InputBotInlineMessageId {
            dc_id: 4,
            id: 1,
            access_hash: 1,
        }
        .into(),
        chat_instance: 1,
        data: Some(vec![1]),
        game_short_name: None,
    }
    .into();
    let Update::CallbackQuery(inline) = Update::from_raw(&client, inline, state, peers) else {
        panic!("inline")
    };
    inline.answer().edit("text only").await.unwrap();
}

async fn participant_checks(client: Client, model: Arc<Mutex<Model>>, peer: PeerRef) {
    let chat = PeerRef {
        id: PeerId::chat_unchecked(10),
        auth: PeerAuth::default(),
    };
    for forbidden in [None, Some(false), Some(true)] {
        model.lock().unwrap().forbidden = forbidden;
        let mut iter = client.iter_participants(chat);
        let expected = usize::from(forbidden != Some(false));
        let before = model.lock().unwrap().calls.len();
        assert_eq!(iter.total().await.unwrap(), expected);
        assert_eq!(iter.total().await.unwrap(), expected);
        while iter.next().await.unwrap().is_some() {}
        assert_eq!(iter.total().await.unwrap(), expected);
        assert!(iter.next().await.unwrap().is_none());
        assert_eq!(model.lock().unwrap().calls.len(), before + 1);
    }
    model.lock().unwrap().forbidden = None;
    let channel = PeerRef {
        id: PeerId::channel_unchecked(10),
        auth: PeerAuth::from_hash(1),
    };
    for group in [channel, chat] {
        model.lock().unwrap().role = 0;
        let builder = client
            .set_admin_rights(group, peer)
            .load_current()
            .await
            .unwrap();
        model.lock().unwrap().role = 1;
        builder.load_current().await.unwrap().await.unwrap();
        assert_eq!(
            model.lock().unwrap().admin_flags.last().unwrap(),
            &(false, String::new())
        );
    }
    model.lock().unwrap().role = 2;
    let builder = client
        .set_banned_rights(channel, peer)
        .load_current()
        .await
        .unwrap();
    model.lock().unwrap().role = 1;
    builder.load_current().await.unwrap().await.unwrap();
    assert_eq!(
        model.lock().unwrap().banned_flags.last().unwrap(),
        &(false, 0)
    );
    client
        .set_banned_rights(channel, peer)
        .duration(Duration::from_secs(u64::MAX))
        .await
        .unwrap();
    assert_eq!(model.lock().unwrap().banned_flags.last().unwrap().1, 0);
}
