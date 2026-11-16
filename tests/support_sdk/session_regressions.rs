#![allow(clippy::unwrap_used)]
use grammers_session::{
    BoxFuture, Session,
    storages::{MemorySession, MemorySessionError, erase},
    types::{ChannelState, DcOption, PeerAuth, PeerId, PeerInfo, UpdateState, UpdatesState},
    updates::{MessageBoxes, UpdatesLike},
};
use grammers_tl_types as tl;
#[path = "session885.rs"]
mod followup885;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Counting {
    inner: MemorySession,
    batches: AtomicUsize,
}
impl Session for Counting {
    type Error = MemorySessionError;
    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        self.inner.home_dc_id()
    }
    fn set_home_dc_id(&self, id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_home_dc_id(id)
    }
    fn dc_option(&self, id: i32) -> Result<Option<DcOption>, Self::Error> {
        self.inner.dc_option(id)
    }
    fn set_dc_option(&self, dc: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_dc_option(dc)
    }
    fn peer(&self, id: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        self.inner.peer(id)
    }
    fn cache_peer(&self, peer: PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.cache_peer(peer)
    }
    fn cache_peers(&self, peers: Vec<PeerInfo>) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.batches.fetch_add(1, Ordering::AcqRel);
        self.inner.cache_peers(peers)
    }
    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        self.inner.updates_state()
    }
    fn set_update_state(&self, state: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        self.inner.set_update_state(state)
    }
}
fn loaded() -> MessageBoxes {
    MessageBoxes::load(UpdatesState {
        pts: 10,
        qts: 40,
        date: 123,
        seq: 1,
        channels: vec![ChannelState { id: 43, pts: 21 }],
    })
}
fn short(update: tl::enums::Update) -> UpdatesLike {
    UpdatesLike::Updates(tl::types::UpdateShort { update, date: 124 }.into())
}
fn empty_channel(timeout: i32) -> tl::enums::updates::ChannelDifference {
    tl::types::updates::ChannelDifferenceEmpty {
        r#final: true,
        pts: 21,
        timeout: Some(timeout),
    }
    .into()
}
pub async fn run() {
    for invalid in [i64::MIN, -1, 0] {
        assert!(PeerId::chat(invalid).is_none());
    }
    assert!(PeerId::channel(i64::MAX).is_none());
    assert_eq!(PeerId::chat(1).unwrap().bare_id(), Some(1));
    let counting = Arc::new(Counting::default());
    let erased = erase(Arc::clone(&counting));
    erased
        .cache_peers(vec![PeerInfo::User {
            id: 7,
            auth: Some(PeerAuth::from_hash(9)),
            bot: Some(false),
            is_self: Some(true),
        }])
        .await
        .unwrap();
    assert_eq!(
        counting.batches.load(Ordering::Acquire),
        1,
        "erasure must preserve the real bulk operation"
    );
    assert!(matches!(
        erased.peer(PeerId::self_user()).await.unwrap(),
        Some(PeerInfo::User { id: 7, .. })
    ));
    for (name, expected, value) in [
        ("2FA_CONFIRM_WAIT_10", "2FA_CONFIRM_WAIT", Some(10)),
        ("SOME_2FA_ERROR", "SOME_2FA_ERROR", None),
        ("FLOOD_12_WAIT_123", "FLOOD_WAIT_123", Some(12)),
    ] {
        let error = grammers_mtsender::RpcError::from(tl::types::RpcError {
            error_code: 400,
            error_message: name.into(),
        });
        assert_eq!(error.name, expected);
        assert_eq!(error.value, value);
    }
    let error = grammers_mtsender::InvocationError::Session(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "session access denied",
    )));
    assert!(error.to_string().contains("session access denied"));
    assert_eq!(
        std::error::Error::source(&error).unwrap().to_string(),
        "session access denied"
    );

    let mut channel = loaded();
    channel
        .process_updates(short(
            tl::types::UpdateChannelTooLong {
                channel_id: 43,
                pts: Some(23),
            }
            .into(),
        ))
        .unwrap();
    assert_eq!(
        channel.get_channel_difference().unwrap().pts,
        21,
        "remote hint cannot skip local channel history"
    );
    channel.apply_channel_difference(empty_channel(1800));
    let checkpoint = channel.durable_checkpoint().unwrap();
    let restored = MessageBoxes::from_durable_checkpoint(&checkpoint).unwrap();
    assert_eq!(restored.session_state().channels[0].pts, 21);
    channel.begin_durable_difference(Some((43, 21))).unwrap();
    channel.apply_channel_difference(empty_channel(1));
    assert!(
        channel
            .check_deadlines()
            .saturating_duration_since(std::time::Instant::now())
            <= std::time::Duration::from_secs(2),
        "shortened channel timeout must update cached minimum"
    );

    let mut machine = loaded();
    machine.begin_durable_difference(None).unwrap();
    let update = tl::types::UpdateDeleteMessages {
        messages: vec![77],
        pts: 11,
        pts_count: 1,
    };
    assert!(
        machine
            .process_updates(short(update.clone().into()))
            .unwrap()
            .0
            .is_empty(),
        "live input must wait for its authoritative difference"
    );
    assert_eq!(machine.session_state().pts, 10);
    let output = machine.apply_difference(
        tl::types::updates::Difference {
            new_messages: vec![],
            new_encrypted_messages: vec![],
            other_updates: vec![update.clone().into()],
            chats: vec![],
            users: vec![],
            state: tl::types::updates::State {
                pts: 11,
                qts: 40,
                date: 124,
                seq: 2,
                unread_count: 0,
            }
            .into(),
        }
        .into(),
    );
    assert_eq!(
        output.0.len(),
        1,
        "authoritative other_updates must still be delivered"
    );
    assert!(
        machine
            .process_updates(short(update.into()))
            .unwrap()
            .0
            .is_empty(),
        "replayed socket input cannot duplicate the difference"
    );
    assert_eq!(machine.session_state().pts, 11);

    let mut machine = loaded();
    let request = tl::functions::messages::SendMessage {
        peer: tl::enums::InputPeer::PeerSelf,
        message: "saved message".into(),
        random_id: 42,
        no_webpage: false,
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        allow_paid_floodskip: false,
        reply_to: None,
        reply_markup: None,
        entities: None,
        schedule_date: None,
        schedule_repeat_period: None,
        send_as: None,
        quick_reply_shortcut: None,
        effect: None,
        allow_paid_stars: None,
        suggested_post: None,
        rich_message: None,
    };
    followup885::run(&request);
    let result = machine.process_updates(UpdatesLike::ShortSentMessage {
        request,
        update: tl::types::UpdateShortSentMessage {
            out: true,
            id: 99,
            pts: 11,
            pts_count: 1,
            date: 124,
            media: None,
            entities: None,
            ttl_period: None,
        },
    });
    assert!(
        result.is_err(),
        "unresolved self must recover rather than fabricate peer zero"
    );
    assert_eq!(machine.get_difference().unwrap().pts, 10);
}
