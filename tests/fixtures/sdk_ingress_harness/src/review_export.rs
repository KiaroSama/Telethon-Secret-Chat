//! Public SDK runtime regression paths through the existing offline I/O fixture.
use kiaro_secret_chat::{
    export::{
        fetch_client::{ExportClient, FetchError},
        grammers_client::GrammersExportClient,
    },
    grammers_client::{
        Client,
        peer::{Role, User},
        update::Update,
    },
    grammers_mtsender::{InvocationError, SenderPool, UpdatesConfiguration},
    grammers_session::{
        SessionData,
        storages::MemorySession,
        types::{PeerAuth, PeerId, PeerRef},
        updates::UpdatesLike,
    },
    grammers_tl_types::{self as tl, Identifiable, Serializable},
};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Model {
    missing: bool,
    empty_message: bool,
    transport: bool,
    dialog_slice: bool,
    calls: Vec<u32>,
}

fn user(id: i64, flags: u32, hash: Option<i64>) -> tl::enums::User {
    let mut bytes = tl::types::User::CONSTRUCTOR_ID.to_le_bytes().to_vec();
    bytes.extend((flags | if hash.is_some() { 1 } else { 0 }).to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(id.to_le_bytes());
    if let Some(hash) = hash {
        bytes.extend(hash.to_le_bytes());
    }
    if flags & (1 << 14) != 0 {
        bytes.extend(0i32.to_le_bytes());
    }
    super::decode(&bytes)
}
fn service(id: i32, peer: i64) -> tl::enums::Message {
    let mut bytes = tl::types::MessageService::CONSTRUCTOR_ID
        .to_le_bytes()
        .to_vec();
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(id.to_le_bytes());
    bytes.extend(tl::types::PeerUser::CONSTRUCTOR_ID.to_le_bytes());
    bytes.extend(peer.to_le_bytes());
    bytes.extend(123i32.to_le_bytes());
    bytes.extend(tl::enums::MessageAction::Empty.to_bytes());
    super::decode(&bytes)
}
fn dialog(peer: i64) -> tl::enums::Dialog {
    let mut bytes = tl::types::Dialog::CONSTRUCTOR_ID.to_le_bytes().to_vec();
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(tl::types::PeerUser::CONSTRUCTOR_ID.to_le_bytes());
    bytes.extend(peer.to_le_bytes());
    for value in [1i32, 0, 0, 0, 0, 0, 0] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(tl::types::PeerNotifySettings::CONSTRUCTOR_ID.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    super::decode(&bytes)
}
fn channel(id: i64) -> tl::enums::Chat {
    tl::types::ChannelForbidden {
        broadcast: true,
        megagroup: false,
        monoforum: false,
        id,
        access_hash: 1,
        title: format!("Public channel {id}"),
        until_date: None,
    }
    .into()
}
impl Model {
    fn invoke(&mut self, body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let id = u32::from_le_bytes(body[..4].try_into().unwrap());
        self.calls.push(id);
        if self.transport {
            return Err(InvocationError::Dropped(
                "public burned transport diagnostic",
            ));
        }
        Ok(match id {
            id if id == tl::functions::messages::SetBotCallbackAnswer::CONSTRUCTOR_ID
                || id == tl::functions::messages::EditInlineBotMessage::CONSTRUCTOR_ID =>
            {
                true.to_bytes()
            }
            id if id == tl::functions::messages::GetMessages::CONSTRUCTOR_ID => {
                tl::enums::messages::Messages::Messages(tl::types::messages::Messages {
                    messages: if self.missing {
                        vec![]
                    } else if self.empty_message {
                        vec![
                            tl::types::MessageEmpty {
                                id: 1,
                                peer_id: None,
                            }
                            .into(),
                        ]
                    } else {
                        vec![service(1, 123)]
                    },
                    topics: vec![],
                    chats: vec![],
                    users: vec![user(123, 0, Some(1))],
                })
                .to_bytes()
            }
            id if id == tl::functions::messages::GetDialogs::CONSTRUCTOR_ID => {
                if self.dialog_slice {
                    tl::enums::messages::Dialogs::Slice(tl::types::messages::DialogsSlice {
                        count: 101,
                        dialogs: (456..556).map(dialog).collect(),
                        messages: (456..556).map(|id| service(1, id)).collect(),
                        chats: vec![],
                        users: (456..556).map(|id| user(id, 1 << 20, None)).collect(),
                    })
                    .to_bytes()
                } else {
                    tl::enums::messages::Dialogs::Dialogs(tl::types::messages::Dialogs {
                        dialogs: vec![dialog(456)],
                        messages: vec![service(1, 456)],
                        chats: vec![],
                        users: vec![user(456, 1 << 20, None)],
                    })
                    .to_bytes()
                }
            }
            id if id == tl::functions::channels::GetParticipants::CONSTRUCTOR_ID => {
                let mut rights = tl::types::ChatBannedRights::CONSTRUCTOR_ID
                    .to_le_bytes()
                    .to_vec();
                rights.extend([0; 8]);
                let banned_rights = super::decode(&rights);
                tl::enums::channels::ChannelParticipants::Participants(
                    tl::types::channels::ChannelParticipants {
                        count: 2,
                        participants: vec![
                            tl::types::ChannelParticipantBanned {
                                left: false,
                                peer: tl::types::PeerChannel { channel_id: 77 }.into(),
                                kicked_by: 123,
                                date: 0,
                                banned_rights,
                                rank: None,
                            }
                            .into(),
                            tl::types::ChannelParticipantLeft {
                                peer: tl::types::PeerChannel { channel_id: 78 }.into(),
                            }
                            .into(),
                        ],
                        chats: vec![channel(77), channel(78)],
                        users: vec![],
                    },
                )
                .to_bytes()
            }
            _ => panic!("unexpected SDK review RPC {id:x}"),
        })
    }
}
pub async fn run() {
    let mut data = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    data.dc_options.get_mut(&2).unwrap().auth_key = Some([7; 256]);
    let pool = SenderPool::new(Arc::new(MemorySession::from(data)), 123);
    let client = Client::new(pool.handle.clone());
    let handle = pool.handle.clone();
    let live = pool.runner.offline_fixture_updates();
    let model = Arc::new(Mutex::new(Model::default()));
    let remote = model.clone();
    let runner = tokio::spawn(pool.runner.run_offline_fixture(move |dc, body| {
        assert_eq!(dc, 2);
        let remote = remote.clone();
        async move { remote.lock().unwrap().invoke(body) }
    }));
    let mut updates = client
        .stream_updates(pool.updates, UpdatesConfiguration::default())
        .await
        .unwrap();
    let callback: tl::enums::Update = tl::types::UpdateBotCallbackQuery {
        query_id: 1,
        user_id: 123,
        peer: tl::types::PeerUser { user_id: 123 }.into(),
        msg_id: 1,
        chat_instance: 1,
        data: None,
        game_short_name: None,
    }
    .into();
    live.send(UpdatesLike::Updates(
        tl::types::Updates {
            updates: vec![callback],
            users: vec![user(123, 0, Some(1))],
            chats: vec![],
            date: 123,
            seq: 1,
        }
        .into(),
    ))
    .await
    .unwrap();
    let (raw, state, peers) = updates.next_raw().await.unwrap();
    let Update::CallbackQuery(query) = Update::from_raw(&client, raw, state.clone(), peers.clone())
    else {
        panic!("callback")
    };
    model.lock().unwrap().missing = true;
    assert!(query.load_message().await.is_err());
    model.lock().unwrap().missing = false;
    model.lock().unwrap().empty_message = true;
    assert!(query.load_message().await.is_err());
    model.lock().unwrap().empty_message = false;
    let message = query.load_message().await.unwrap();
    assert_eq!(message.refetch().await.unwrap().unwrap().id(), 1);
    model.lock().unwrap().empty_message = true;
    assert!(message.refetch().await.unwrap().is_none());
    model.lock().unwrap().empty_message = false;
    model.lock().unwrap().missing = true;
    assert!(message.refetch().await.unwrap().is_none());
    assert_eq!(message.id(), 1);
    model.lock().unwrap().missing = false;
    let inline: tl::enums::Update = tl::types::UpdateInlineBotCallbackQuery {
        query_id: 2,
        user_id: 999,
        msg_id: tl::types::InputBotInlineMessageId {
            dc_id: 2,
            id: 1,
            access_hash: 1,
        }
        .into(),
        chat_instance: 1,
        data: None,
        game_short_name: None,
    }
    .into();
    let Update::CallbackQuery(inline) =
        Update::from_raw(&client, inline, state.clone(), peers.clone())
    else {
        panic!("inline callback")
    };
    let count = model.lock().unwrap().calls.len();
    assert!(inline.answer().respond("public").await.is_err());
    assert_eq!(model.lock().unwrap().calls.len(), count);
    inline.answer().edit("public edited").await.unwrap();
    assert_eq!(
        &model.lock().unwrap().calls[count..],
        &[
            tl::functions::messages::SetBotCallbackAnswer::CONSTRUCTOR_ID,
            tl::functions::messages::EditInlineBotMessage::CONSTRUCTOR_ID
        ]
    );
    let dialog = client.iter_dialogs().next().await.unwrap().unwrap();
    assert_eq!(dialog.peer_id(), PeerId::user_unchecked(456));
    assert!(dialog.peer_ref().is_none());
    assert_eq!(
        dialog.last_message.unwrap().peer_id(),
        PeerId::user_unchecked(456)
    );
    model.lock().unwrap().dialog_slice = true;
    assert!(matches!(
        client.iter_dialogs().next().await,
        Err(InvocationError::Dropped(_))
    ));
    model.lock().unwrap().dialog_slice = false;
    let mut members = client.iter_participants(PeerRef {
        id: PeerId::channel_unchecked(42),
        auth: PeerAuth::from_hash(1),
    });
    let banned = members.next().await.unwrap().unwrap();
    assert_eq!(banned.peer.id(), PeerId::channel_unchecked(77));
    assert!(banned.user().is_none() && matches!(banned.role, Role::Banned(_)));
    let left = members.next().await.unwrap().unwrap();
    assert_eq!(left.peer.id(), PeerId::channel_unchecked(78));
    assert!(left.user().is_none() && matches!(left.role, Role::Left(_)));
    for (flags, expected) in [(1 << 14, true), ((1 << 14) | (1 << 16), false), (0, false)] {
        assert_eq!(
            User::from_raw(&client, user(800, flags, None)).bot_supports_chats(),
            expected
        );
    }
    let empty: tl::enums::Message = tl::types::MessageEmpty {
        id: 1,
        peer_id: None,
    }
    .into();
    let edit = tl::types::UpdateEditChannelMessage {
        message: empty.clone(),
        pts: 1,
        pts_count: 1,
    }
    .into();
    assert!(matches!(
        Update::from_raw(&client, edit, state.clone(), peers.clone()),
        Update::Raw(_)
    ));
    for (message, refs) in [(empty.clone(), None), (service(1, 123), Some(vec![empty]))] {
        let guest = tl::types::UpdateBotGuestChatQuery {
            query_id: 3,
            message,
            reference_messages: refs,
            qts: 1,
        }
        .into();
        assert!(matches!(
            Update::from_raw(&client, guest, state.clone(), peers.clone()),
            Update::Raw(_)
        ));
    }
    model.lock().unwrap().transport = true;
    let error = GrammersExportClient::new(client)
        .request(
            Default::default(),
            serde_json::json!({"_":"users.GetUsers","id":[{"_":"InputUserSelf"}]}),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, FetchError::Transport));
    assert!(!format!("{error:?}").contains("burned"));
    handle.quit();
    runner.await.unwrap();
}
