#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::export::{
    error::ExportError,
    format::DateContext,
    json_message::serialize_message,
    json_serialize::JsonContext,
    json_values::MessageValues,
    model::{File, Image, Photo, SkipReason, peer_from_channel, peer_from_chat, peer_from_user},
    model_actions::{Action, ActionChatEditPhoto, ServiceAction},
    model_message::Message,
    model_peers::{ContactInfo, Peer, PeerUser, Peers},
};
#[test]
fn file_path_placeholders_and_unexpected_skip_reason() {
    for (reason, text) in [
        (
            SkipReason::FileSize,
            "(File exceeds maximum size. Change data exporting settings",
        ),
        (
            SkipReason::FileType,
            "(File not included. Change data exporting settings",
        ),
    ] {
        let message = Message {
            id: 7,
            date: 1705053605,
            action: ServiceAction {
                content: Some(Action::ChatEditPhoto(ActionChatEditPhoto {
                    photo: Photo {
                        image: Image {
                            file: File {
                                skip_reason: reason,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                })),
            },
            ..Default::default()
        };
        let mut context = JsonContext {
            nesting: vec![true, false],
            ..Default::default()
        };
        let bytes = serialize_message(
            &mut context,
            &message,
            &Peers::new(),
            "",
            &DateContext::utc(),
        )
        .unwrap();
        assert!(bytes.windows(text.len()).any(|w| w == text.as_bytes()));
    }
    let message = Message {
        action: ServiceAction {
            content: Some(Action::ChatEditPhoto(ActionChatEditPhoto {
                photo: Photo {
                    image: Image {
                        file: File {
                            skip_reason: SkipReason::DateLimits,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                },
            })),
        },
        ..Default::default()
    };
    let mut context = JsonContext {
        nesting: vec![true, false],
        ..Default::default()
    };
    assert!(matches!(
        serialize_message(
            &mut context,
            &message,
            &Peers::new(),
            "",
            &DateContext::utc()
        ),
        Err(ExportError::InvalidRecord)
    ));
    assert_eq!(context.nesting, vec![true, false]);
}
#[test]
fn peer_ids_and_names_of_unknown_peers() {
    assert_eq!(MessageValues::wrap_peer_id(peer_from_chat(4)), b"\"chat4\"");
    assert_eq!(
        MessageValues::wrap_peer_id(peer_from_channel(4)),
        b"\"channel4\""
    );
    assert_eq!(MessageValues::wrap_peer_id(peer_from_user(4)), b"\"user4\"");
    let mut context = JsonContext::default();
    let message = Message::default();
    let empty = Peers::new();
    let values = MessageValues::new(&mut context, &message, &empty);
    assert_eq!(values.wrap_peer_name(peer_from_user(4)), b"null");
    let mut peers = Peers::new();
    peers.insert(
        peer_from_user(100),
        Peer::User(PeerUser {
            bare_id: 100,
            info: ContactInfo {
                user_id: 100,
                first_name: "Ann".into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    );
    let mut values = MessageValues::new(&mut context, &message, &peers);
    assert_eq!(values.wrap_user_name(9), b"null");
    assert_eq!(values.wrap_user_name(100), b"\"Ann\"");
    values.string("empty", "");
    values.bare("bare", vec![]);
    assert!(values.values.is_empty());
}
