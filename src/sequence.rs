//! In-order receive, gap recovery, mirrored counters and exact-byte resends.
use crate::{
    chat::{Chat, Envelope, Outgoing},
    error::{Error, Result},
    types::DecryptedMessageLayer,
};
pub const MAX_RESEND_COUNT: i32 = 1000;
pub struct Accepted {
    pub ready: Vec<(i32, Envelope)>,
    pub resend: Option<(i32, i32)>,
    pub acknowledged: Vec<i64>,
}
fn check_order(chat: &Chat, wrapper: &DecryptedMessageLayer) -> Result<()> {
    let peer_in = wrapper.in_seq_no / 2;
    if peer_in < chat.peer_in_seq_no
        || peer_in > chat.out_seq_no
        || wrapper.layer < chat.wrapper_layer
    {
        return Err(Error::Sequence);
    }
    Ok(())
}
pub fn preflight(chat: &Chat, wrapper: &DecryptedMessageLayer) -> Result<bool> {
    preflight_header(
        chat,
        &crate::framing::Header {
            layer: wrapper.layer,
            in_seq_no: wrapper.in_seq_no,
            out_seq_no: wrapper.out_seq_no,
            body_offset: 0,
        },
    )
}
pub fn preflight_header(chat: &Chat, wrapper: &crate::framing::Header) -> Result<bool> {
    if wrapper.in_seq_no < 0
        || wrapper.out_seq_no < 0
        || wrapper.in_seq_no % 2 != i32::from(chat.is_outbound)
        || wrapper.out_seq_no % 2 != i32::from(!chat.is_outbound)
    {
        return Err(Error::Sequence);
    }
    let raw = wrapper.out_seq_no / 2;
    if raw < chat.in_seq_no || chat.gaps.contains_key(&raw) {
        return Ok(false);
    }
    if wrapper.in_seq_no / 2 < chat.peer_in_seq_no
        || wrapper.in_seq_no / 2 > chat.out_seq_no
        || wrapper.layer < chat.wrapper_layer
    {
        return Err(Error::Sequence);
    }
    if raw - chat.in_seq_no > MAX_RESEND_COUNT
        || (raw > chat.in_seq_no && chat.gaps.len() >= MAX_RESEND_COUNT as usize)
    {
        return Err(Error::Sequence);
    }
    Ok(true)
}
pub fn accept(chat: &mut Chat, envelope: Envelope) -> Result<Accepted> {
    let mut result = Accepted {
        ready: Vec::new(),
        resend: None,
        acknowledged: Vec::new(),
    };
    if !preflight_header(chat, &crate::framing::inspect_header(&envelope.body)?)? {
        return Ok(result);
    }
    let wrapper = crate::framing::unwrap(&envelope.body)?;
    chat.capability_layer = chat.capability_layer.max(wrapper.layer);
    let raw = wrapper.out_seq_no / 2;
    if raw > chat.in_seq_no {
        chat.gaps.insert(raw, envelope);
        if chat.gap_end.is_none() {
            chat.gap_end = Some(raw);
            result.resend = Some((
                2 * chat.in_seq_no + wrapper.out_seq_no % 2,
                wrapper.out_seq_no - 2,
            ));
            chat.resend_due = result.resend;
        }
        return Ok(result);
    }
    let mut current = Some(envelope);
    while let Some(envelope) = current {
        let wrapper = crate::framing::unwrap(&envelope.body)?;
        check_order(chat, &wrapper)?;
        chat.peer_in_seq_no = wrapper.in_seq_no / 2;
        chat.wrapper_layer = wrapper.layer;
        let sequence = chat.in_seq_no;
        chat.in_seq_no = chat
            .in_seq_no
            .checked_add(1)
            .filter(|n| *n <= i32::MAX / 2)
            .ok_or(Error::Sequence)?;
        result.ready.push((sequence, envelope));
        current = chat.gaps.remove(&chat.in_seq_no);
    }
    if chat.gaps.is_empty() {
        chat.gap_end = None;
        chat.resend_due = None;
    } else if chat.gap_end.is_none_or(|end| chat.in_seq_no >= end) {
        let end = *chat.gaps.first_key_value().ok_or(Error::Sequence)?.0;
        chat.gap_end = Some(end);
        result.resend = Some((
            2 * chat.in_seq_no + wrapper.out_seq_no % 2,
            2 * (end - 1) + wrapper.out_seq_no % 2,
        ));
        chat.resend_due = result.resend;
    }
    Ok(result)
}
pub fn answer_resend(chat: &Chat, start: i32, end: i32) -> Result<Vec<Outgoing>> {
    let parity = i32::from(chat.is_outbound);
    if start < 0
        || end < start
        || start % 2 != parity
        || end % 2 != parity
        || (end - start) / 2 + 1 > MAX_RESEND_COUNT
    {
        return Err(Error::ResendUnavailable);
    }
    (start..=end)
        .step_by(2)
        .map(|seq| {
            chat.outbox
                .get(&seq)
                .cloned()
                .ok_or(Error::ResendUnavailable)
        })
        .collect()
}
