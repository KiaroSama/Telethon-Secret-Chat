//! Independent state-machine and malformed-checkpoint review controls.
#![allow(clippy::unwrap_used)]
use super::{empty_channel, loaded, short};
use grammers_session::{
    types::{ChannelState, UpdatesState},
    updates::{MessageBox, MessageBoxes, PrematureEndReason, UpdatesLike},
};
use grammers_tl_types::{self as tl, Serializable};
fn common_only() -> MessageBoxes {
    let mut machine = MessageBoxes::load(UpdatesState {
        pts: 10,
        date: 123,
        seq: 1,
        ..Default::default()
    });
    machine.apply_difference(tl::types::updates::DifferenceEmpty { date: 123, seq: 1 }.into());
    machine
}
pub fn run(request: &tl::functions::messages::SendMessage) {
    type Check = fn(&tl::functions::messages::SendMessage);
    let checks: &[(&str, Check)] = &[
        ("secondary_without_common", |_| {
            let mut machine = MessageBoxes::load(UpdatesState {
                qts: 40,
                date: 123,
                seq: 1,
                ..Default::default()
            });
            machine.begin_durable_difference(None).unwrap();
            let difference = machine.get_difference().unwrap();
            assert_eq!((difference.pts, difference.qts), (0, 40));
        }),
        ("final_channel_authority", |_| {
            let mut machine = MessageBoxes::load(UpdatesState {
                pts: 10,
                qts: 40,
                date: 123,
                seq: 1,
                channels: vec![
                    ChannelState { id: 43, pts: 21 },
                    ChannelState { id: 44, pts: 30 },
                ],
            });
            let channel = tl::types::UpdateDeleteChannelMessages {
                channel_id: 43,
                messages: vec![7],
                pts: 22,
                pts_count: 1,
            };
            let output = machine.apply_channel_difference(
                tl::types::updates::ChannelDifference {
                    r#final: true,
                    pts: 22,
                    timeout: None,
                    new_messages: vec![],
                    other_updates: vec![
                        channel.clone().into(),
                        tl::types::UpdateDeleteChannelMessages {
                            channel_id: 44,
                            messages: vec![8],
                            pts: 31,
                            pts_count: 1,
                        }
                        .into(),
                        tl::types::UpdateDeleteMessages {
                            messages: vec![9],
                            pts: 11,
                            pts_count: 1,
                        }
                        .into(),
                    ],
                    chats: vec![],
                    users: vec![],
                }
                .into(),
            );
            assert_eq!(output.0.len(), 1);
            assert!(
                matches!(&output.0[0].0,tl::enums::Update::DeleteChannelMessages(update) if update.channel_id==43 && update.messages==[7])
            );
            assert!(matches!(
                output.0[0].1.message_box,
                Some(MessageBox::Channel {
                    channel_id: 43,
                    pts: 22
                })
            ));
            assert_eq!(machine.session_state().channels[1].pts, 30);
            assert!(
                machine
                    .process_updates(short(channel.into()))
                    .unwrap()
                    .0
                    .is_empty()
            );
        }),
        ("account_secondary_authority", |_| {
            let mut machine = MessageBoxes::new();
            machine.set_state(tl::types::updates::State {
                pts: 10,
                qts: 40,
                date: 123,
                seq: 1,
                unread_count: 0,
            });
            assert!(
                machine
                    .process_updates(UpdatesLike::ConnectionClosed)
                    .is_err()
            );
            let update = tl::types::UpdateChannelParticipant {
                via_chatlist: false,
                channel_id: 43,
                date: 124,
                actor_id: 7,
                user_id: 8,
                prev_participant: None,
                new_participant: None,
                invite: None,
                qts: 41,
            };
            let output = machine.apply_difference(
                tl::types::updates::Difference {
                    new_messages: vec![],
                    new_encrypted_messages: vec![],
                    other_updates: vec![update.clone().into()],
                    chats: vec![],
                    users: vec![],
                    state: tl::types::updates::State {
                        pts: 10,
                        qts: 41,
                        date: 124,
                        seq: 2,
                        unread_count: 0,
                    }
                    .into(),
                }
                .into(),
            );
            assert_eq!(output.0.len(), 1);
            assert!(matches!(
                output.0[0].1.message_box,
                Some(MessageBox::Secondary { qts: 41 })
            ));
            assert!(
                machine
                    .process_updates(short(update.into()))
                    .unwrap()
                    .0
                    .is_empty()
            );
        }),
        ("channel_short_sent", |request| {
            for peer in [
                tl::types::InputPeerChannel {
                    channel_id: 43,
                    access_hash: 1,
                }
                .into(),
                tl::types::InputPeerChannelFromMessage {
                    peer: tl::types::InputPeerUser {
                        user_id: 123,
                        access_hash: 1,
                    }
                    .into(),
                    msg_id: 7,
                    channel_id: 43,
                }
                .into(),
            ] {
                let mut machine = loaded();
                machine.apply_channel_difference(empty_channel(900));
                let mut request = request.clone();
                request.peer = peer;
                let output = machine
                    .process_updates(UpdatesLike::ShortSentMessage {
                        request,
                        update: tl::types::UpdateShortSentMessage {
                            out: true,
                            id: 99,
                            pts: 22,
                            pts_count: 1,
                            date: 124,
                            media: None,
                            entities: None,
                            ttl_period: None,
                        },
                    })
                    .unwrap();
                assert_eq!(output.0.len(), 1);
                assert!(
                    matches!(&output.0[0].0,tl::enums::Update::NewChannelMessage(update) if update.pts==22)
                );
                assert_eq!(machine.session_state().channels[0].pts, 22);
                assert_eq!(machine.session_state().pts, 10);
            }
        }),
        ("checkpoint_gap_owner", |_| {
            let mut machine = common_only();
            let update: tl::enums::Update = tl::types::UpdateDeleteMessages {
                messages: vec![7],
                pts: 12,
                pts_count: 1,
            }
            .into();
            assert!(
                machine
                    .process_updates(short(update.clone()))
                    .unwrap()
                    .0
                    .is_empty()
            );
            let original = machine.durable_checkpoint().unwrap();
            assert!(MessageBoxes::from_durable_checkpoint(&original).is_ok());
            let payload = update.to_bytes();
            let at = original
                .windows(payload.len())
                .position(|bytes| bytes == payload)
                .unwrap();
            assert_eq!(
                i32::from_le_bytes(original[at - 4..at].try_into().unwrap()) as usize,
                payload.len()
            );
            for invalid in [
                tl::types::UpdateEncryptedChatTyping { chat_id: 7 }.into(),
                tl::types::UpdateDeleteChannelMessages {
                    channel_id: 43,
                    messages: vec![7],
                    pts: 12,
                    pts_count: 1,
                }
                .into(),
                tl::types::UpdateNewMessage {
                    message: tl::types::MessageEmpty {
                        id: 7,
                        peer_id: Some(tl::types::PeerChannel { channel_id: 43 }.into()),
                    }
                    .into(),
                    pts: 12,
                    pts_count: 1,
                }
                .into(),
            ] {
                let payload = tl::enums::Update::to_bytes(&invalid);
                let mut modified = original.clone();
                modified.splice(
                    at - 4..at + update.to_bytes().len(),
                    (payload.len() as i32)
                        .to_le_bytes()
                        .into_iter()
                        .chain(payload),
                );
                assert!(MessageBoxes::from_durable_checkpoint(&modified).is_err());
            }
        }),
        ("stale_cached_deadline", |_| {
            let mut checkpoint = common_only().durable_checkpoint().unwrap();
            assert_eq!(
                i32::from_le_bytes(checkpoint[24..28].try_into().unwrap()),
                1
            );
            assert_eq!(
                i32::from_le_bytes(checkpoint[28..32].try_into().unwrap()),
                0
            );
            assert_eq!(
                i32::from_le_bytes(checkpoint[32..36].try_into().unwrap()),
                10
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            checkpoint[16..24].copy_from_slice(&(now - 1).to_le_bytes());
            checkpoint[36..44].copy_from_slice(&(now + 10_000).to_le_bytes());
            let mut machine = MessageBoxes::from_durable_checkpoint(&checkpoint).unwrap();
            let wait = machine
                .check_deadlines()
                .saturating_duration_since(std::time::Instant::now());
            assert!(
                wait <= std::time::Duration::from_secs(20),
                "stale deadline postponed the pending entry: {wait:?}"
            );
        }),
        ("stale_channel_error_is_scoped", |_| {
            let mut machine = MessageBoxes::load(UpdatesState {
                pts: 10,
                qts: 40,
                date: 123,
                seq: 1,
                channels: vec![
                    ChannelState { id: 43, pts: 21 },
                    ChannelState { id: 44, pts: 30 },
                ],
            });
            machine.end_channel_difference_for(43, PrematureEndReason::TemporaryServerIssues);
            machine.end_channel_difference_for(43, PrematureEndReason::Banned);
            let request = machine.get_channel_difference().unwrap();
            assert!(
                matches!(request.channel,tl::enums::InputChannel::Channel(channel) if channel.channel_id==44)
            );
            assert_eq!(
                machine.session_state().channels,
                [
                    ChannelState { id: 43, pts: 21 },
                    ChannelState { id: 44, pts: 30 }
                ]
            );
        }),
    ];
    let failures = checks
        .iter()
        .filter_map(|(name, check)| {
            std::panic::catch_unwind(|| check(request))
                .err()
                .map(|_| *name)
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "Session state regressions: {failures:?}"
    );
}
