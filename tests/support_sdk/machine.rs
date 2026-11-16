#![allow(clippy::unwrap_used)]
use grammers_session::{
    types::UpdatesState,
    updates::{MessageBoxes, UpdatesLike},
};
use grammers_tl_types as tl;

fn machine() -> MessageBoxes {
    MessageBoxes::load(UpdatesState {
        pts: 10,
        qts: 40,
        date: 123,
        seq: 1,
        channels: vec![],
    })
}

pub fn checkpoint_restores_counter_and_difference_ownership() {
    let original = machine();
    let saved = original.durable_checkpoint().unwrap();
    let restored = MessageBoxes::from_durable_checkpoint(&saved).unwrap();
    let actual = restored.get_difference().unwrap();
    assert_eq!((actual.pts, actual.qts, actual.date), (10, 40, 123));
    assert_eq!(actual, original.get_difference().unwrap());
}

pub fn checkpoint_rejects_trailing_bytes_and_truncation() {
    let saved = machine().durable_checkpoint().unwrap();
    for end in 0..saved.len() {
        assert!(MessageBoxes::from_durable_checkpoint(&saved[..end]).is_err());
    }
    let mut extra = saved;
    extra.push(0);
    assert!(MessageBoxes::from_durable_checkpoint(&extra).is_err());
}

pub fn live_codec_retains_complete_envelope_and_peers() {
    let update = UpdatesLike::Updates(tl::enums::Updates::Updates(tl::types::Updates {
        updates: vec![
            tl::types::UpdateNewEncryptedMessage {
                message: tl::types::EncryptedMessageService {
                    random_id: 11,
                    chat_id: 7,
                    date: 123,
                    bytes: b"offline-full-envelope".to_vec(),
                }
                .into(),
                qts: 41,
            }
            .into(),
        ],
        users: vec![tl::types::UserEmpty { id: 33 }.into()],
        chats: vec![tl::types::ChatEmpty { id: 44 }.into()],
        date: 124,
        seq: 2,
    }));
    let bytes = grammers_session::updates::durable::encode_live(&update).unwrap();
    let decoded = grammers_session::updates::durable::decode_live(&bytes).unwrap();
    match (&update, &decoded) {
        (UpdatesLike::Updates(original), UpdatesLike::Updates(restored)) => {
            use grammers_tl_types::Serializable;
            assert_eq!(
                original.to_bytes(),
                restored.to_bytes(),
                "the original TL body and every peer must survive the codec"
            );
        }
        _ => panic!("the codec changed the original envelope kind"),
    }
    assert_eq!(
        grammers_session::updates::durable::encode_live(&decoded).unwrap(),
        bytes
    );
    let mut extra = bytes;
    extra.push(0);
    assert!(grammers_session::updates::durable::decode_live(&extra).is_err());
}
