use super::*;
use crate::{chat::Outgoing, crypto::encrypt_frame};
pub(super) fn prepare(
    chat: &mut Chat,
    message: enums::DecryptedMessage,
    service: bool,
    file: Option<Vec<u8>>,
) -> Result<SendEffect> {
    chat.require_ready()?;
    let random_id = message.random_id();
    let sequence = framing::transform_out_seq_no(chat.out_seq_no, chat.is_outbound)?;
    let wrapper = framing::wrap(
        message,
        framing::outgoing_layer(chat.capability_layer),
        framing::transform_in_seq_no(chat.in_seq_no, chat.is_outbound)?,
        sequence,
    )?;
    let body = wrapper.to_bytes();
    let frame = encrypt_frame(
        chat.key.as_ref().ok_or(Error::ChatState)?,
        &body,
        chat.out_direction(),
    )?;
    let outgoing = Outgoing {
        seq_no: sequence,
        random_id,
        frame,
        body,
        failures: 0,
        service,
        file,
        pending: true,
    };
    chat.out_seq_no = chat
        .out_seq_no
        .checked_add(1)
        .filter(|n| *n <= i32::MAX / 2)
        .ok_or(Error::Sequence)?;
    chat.messages_since_rekey = chat.messages_since_rekey.saturating_add(1);
    chat.outbox.insert(sequence, outgoing.clone());
    Ok(SendEffect {
        chat_id: chat.id,
        access_hash: chat.access_hash,
        outgoing,
    })
}
pub(crate) fn prepare_action(
    chat: &mut Chat,
    action: enums::DecryptedMessageAction,
) -> Result<SendEffect> {
    if let enums::DecryptedMessageAction::SetMessageTtl(ttl) = &action {
        if ttl.ttl_seconds < 0 {
            return Err(Error::Media);
        }
        chat.ttl = ttl.ttl_seconds;
    }

    let message = types::DecryptedMessageService {
        random_id: random_id()?,
        action,
    };
    prepare(chat, message.into(), true, None)
}
pub(super) fn apply_action(
    chat: &mut Chat,
    action: &enums::DecryptedMessageAction,
    effects: &mut Effects,
) -> Result<bool> {
    if matches!(
        action,
        enums::DecryptedMessageAction::RequestKey(_)
            | enums::DecryptedMessageAction::AcceptKey(_)
            | enums::DecryptedMessageAction::CommitKey(_)
            | enums::DecryptedMessageAction::AbortKey(_)
    ) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::ChatState)?
            .as_secs();
        return crate::rekey::handle(chat, action, effects, now);
    }
    match action {
        enums::DecryptedMessageAction::NotifyLayer(layer) => {
            if layer.layer < 0 {
                return Err(Error::WrapperRejected);
            }
            chat.capability_layer = chat.capability_layer.max(layer.layer);
            effects.events.push(Event::LayerChanged {
                chat_id: chat.id,
                layer: chat.capability_layer,
            });
        }
        enums::DecryptedMessageAction::SetMessageTtl(ttl) => {
            if ttl.ttl_seconds < 0 {
                return Err(Error::Sequence);
            }
            chat.ttl = ttl.ttl_seconds;
        }
        enums::DecryptedMessageAction::Resend(_) => return Ok(false),
        enums::DecryptedMessageAction::FlushHistory => {
            let ids = chat
                .outbox
                .values()
                .filter(|row| !row.service)
                .map(|row| row.random_id)
                .collect();
            rewrite_retained_as_deletes(chat, &ids)?;
        }
        enums::DecryptedMessageAction::DeleteMessages(delete) => {
            rewrite_retained_as_deletes(chat, &delete.random_ids.iter().copied().collect())?
        }
        _ => return Ok(false),
    }
    Ok(true)
}
