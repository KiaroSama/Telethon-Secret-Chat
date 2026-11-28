use super::*;
use crate::crypto::encrypt_frame;
use std::collections::BTreeSet;
/// Withdraw content without losing sequence/RPC identity or protocol service work.
pub(crate) fn rewrite_retained_as_deletes(chat: &mut Chat, ids: &BTreeSet<i64>) -> Result<()> {
    let direction = chat.out_direction();
    let key = chat.key.as_ref().ok_or(Error::ChatState)?.clone();
    for row in chat.outbox.values_mut() {
        if row.service || !ids.contains(&row.random_id) {
            continue;
        }
        if row.body.is_empty() {
            return Err(Error::ResendUnavailable);
        }
        let mut wrapper = framing::unwrap(&row.body)?;
        wrapper.message = types::DecryptedMessageService {
            random_id: row.random_id,
            action: types::DecryptedMessageActionDeleteMessages {
                random_ids: vec![row.random_id],
            }
            .into(),
        }
        .into();
        row.body = enums::DecryptedMessageLayer::from(wrapper).to_bytes();
        row.frame = encrypt_frame(&key, &row.body, direction)?;
        row.file = None;
        row.service = true;
    }
    Ok(())
}
