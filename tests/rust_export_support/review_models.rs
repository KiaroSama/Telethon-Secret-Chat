//! Independent model/refusal regressions from the runtime review, inside existing scenarios.
#![allow(clippy::unwrap_used, dead_code)]
use kiaro_secret_chat::export::{
    format::DateContext, model::*, model_actions::*, model_dialogs::*, model_message::*,
    model_peers::*, parse_actions::parse_service_action,
};
use serde_json::json;
pub fn peers() {
    let empty = parse_user(&json!({"_":"UserEmpty","id":42}));
    assert_eq!(empty.id(), peer_from_user(42));
    assert_eq!(dialog_info_from_user(&empty).peer_id, peer_from_user(42));
    for tag in ["Community", "CommunityForbidden", "ChannelForbidden"] {
        let peer = parse_chat(&json!({"_":tag,"id":42}));
        assert_eq!(peer.id(), peer_from_channel(42));
    }
    let missing = empty_peer(peer_from_channel(42));
    assert_eq!(missing.id(), peer_from_channel(42));
    assert_eq!(missing.chat().unwrap().bare_id, 42);
    let map = parse_peers_lists(
        &[],
        &[
            json!({"_":"Chat","id":42}),
            json!({"_":"Community","id":42}),
        ],
    );
    assert_eq!(map.len(), 2);
    assert!(map.contains_key(&peer_from_chat(42)));
    assert!(map.contains_key(&peer_from_channel(42)));
}
pub fn split_counts() {
    let mut info = DialogsInfo {
        chats: vec![DialogInfo {
            r#type: DialogType::Personal,
            splits: vec![0, -2, -1],
            messages_count_per_split: vec![100, 5, 0],
            ..Default::default()
        }],
        ..Default::default()
    };
    finalize_dialogs_info(&mut info, &Default::default()).unwrap();
    assert_eq!(info.chats[0].splits, [-2, -1, 0]);
    assert_eq!(info.chats[0].messages_count_per_split, [5, 0, 100]);
}
pub fn messages() {
    let mut context = ParseMediaContext::default();
    let message=parse_message(&mut context,&json!({"_":"Message","id":10,"date":1700000000,"peer_id":{"_":"PeerChat","chat_id":42},"reply_to":{"_":"MessageReplyHeader","reply_to_msg_id":9,"reply_to_peer_id":{"_":"PeerChat","chat_id":42}}}),"",&DateContext::utc()).unwrap();
    assert_eq!(message.reply_to_peer_id, 0);
    let mut slice = MessagesSlice {
        list: vec![message],
        ..Default::default()
    };
    adjust_migrate_message_ids(&mut slice);
    assert_eq!(slice.list[0].id - slice.list[0].reply_to_msg_id, 1);
    let message=parse_message(&mut context,&json!({"_":"Message","id":1,"date":1700000000,"from_id":{"_":"PeerUser","user_id":42},"peer_id":{"_":"PeerUser","user_id":42},"media":{"_":"MessageMediaGame","game":{"_":"Game","id":7,"title":"Public game","short_name":"public"}}}),"",&DateContext::utc()).unwrap();
    let Some(kiaro_secret_chat::export::model_media::MediaContent::Game(game)) =
        message.media.content
    else {
        panic!("game missing")
    };
    assert_eq!(game.bot_id, 42);
    for (input, expected) in [
        (
            json!({"_":"MessageActionTopicEdit","title":"Renamed"}),
            None,
        ),
        (
            json!({"_":"MessageActionTopicEdit","icon_emoji_id":0}),
            Some(0),
        ),
    ] {
        let action =
            parse_service_action(&mut context, &input, "", 1700000000, &DateContext::utc())
                .unwrap();
        let Some(Action::TopicEdit(topic)) = action.content else {
            panic!("topic edit missing")
        };
        assert_eq!(topic.icon_emoji_id, expected);
    }
}
