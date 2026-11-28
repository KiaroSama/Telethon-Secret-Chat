//! Preserve the authenticated key boundary across gaps, commitments and restarts.
use super::*;

fn buffered_fingerprint(chat: &Chat, sequence: i32) -> Result<i64> {
    if let Some(fingerprint) = chat.gap_key_fingerprints.get(&sequence) {
        return Ok(*fingerprint);
    }
    // Before adoption, the historical receive path always used the current
    // key for retained gaps. Once two keys exist, missing provenance is ambiguous.
    if chat.previous_key.is_none() {
        return chat
            .key
            .as_ref()
            .map(SharedKey::fingerprint)
            .ok_or(Error::InvalidState);
    }
    Err(Error::InvalidState)
}

pub(crate) fn fill_gap_provenance(chat: &mut Chat) -> Result<()> {
    let missing = chat
        .gaps
        .keys()
        .copied()
        .filter(|sequence| !chat.gap_key_fingerprints.contains_key(sequence))
        .map(|sequence| buffered_fingerprint(chat, sequence).map(|key| (sequence, key)))
        .collect::<Result<Vec<_>>>()?;
    chat.gap_key_fingerprints.extend(missing);
    Ok(())
}

pub(crate) fn check_buffered_key(chat: &Chat, sequence: i32, fingerprint: i64) -> Result<()> {
    if chat.gaps.contains_key(&sequence) && buffered_fingerprint(chat, sequence)? != fingerprint {
        return Err(Error::Sequence);
    }
    Ok(())
}

pub(crate) fn received_key_cutover(
    chat: &Chat,
    sequence: i32,
    fingerprint: i64,
    action: Option<&Action>,
) -> Result<Option<i32>> {
    let pending = match &chat.exchange {
        Some(Exchange::Accepted { id, key }) => Some((*id, key)),
        _ => None,
    };
    let mut cutover = chat.incoming_key_cutover;
    let mut tighten = |boundary| {
        cutover = Some(cutover.map_or(boundary, |current| current.min(boundary)));
    };
    if pending.is_some_and(|(_, key)| key.fingerprint() == fingerprint)
        || chat.previous_key.is_some()
            && chat
                .key
                .as_ref()
                .is_some_and(|key| key.fingerprint() == fingerprint)
    {
        tighten(sequence);
    }
    if let Some(Action::CommitKey(commit)) = action {
        let expected = if let Some((_, key)) = pending.filter(|(id, _)| *id == commit.exchange_id) {
            Some((key, chat.key.as_ref()))
        } else if chat.accepted_exchange == Some(commit.exchange_id) {
            chat.key
                .as_ref()
                .map(|key| (key, chat.previous_key.as_ref()))
        } else {
            None
        };
        if let Some((key, preceding)) = expected {
            if preceding.is_none_or(|key| key.fingerprint() != fingerprint) {
                return Err(Error::Sequence);
            }
            verify_fingerprint(key, commit.key_fingerprint).map_err(|_| Error::Sequence)?;
            tighten(sequence.checked_add(1).ok_or(Error::Sequence)?);
        }
    }
    let preceding = chat
        .previous_key
        .as_ref()
        .or_else(|| pending.and(chat.key.as_ref()));
    if let (Some(boundary), Some(preceding)) = (cutover, preceding) {
        let preceding = preceding.fingerprint();
        if fingerprint == preceding && sequence >= boundary {
            return Err(Error::Sequence);
        }
        for &raw in chat.gaps.keys() {
            if raw >= boundary && buffered_fingerprint(chat, raw)? == preceding {
                return Err(Error::Sequence);
            }
        }
    }
    Ok(cutover)
}

pub(crate) fn validate_provenance(chat: &Chat) -> Result<()> {
    let accepted = matches!(chat.exchange, Some(Exchange::Accepted { .. }));
    if chat
        .gap_key_fingerprints
        .keys()
        .any(|raw| !chat.gaps.contains_key(raw))
        || chat.accepted_exchange.is_some() && chat.previous_key.is_none()
        || chat.incoming_key_cutover.is_some_and(|boundary| {
            !(0..=crate::sequence::MAX_SEQUENCE_COUNT).contains(&boundary)
                || chat.previous_key.is_none() && !accepted
        })
    {
        return Err(Error::InvalidState);
    }
    let preceding = chat
        .previous_key
        .as_ref()
        .or(if accepted { chat.key.as_ref() } else { None });
    for &raw in chat.gaps.keys() {
        let fingerprint = buffered_fingerprint(chat, raw)?;
        if select_key(chat, fingerprint).is_none() {
            return Err(Error::InvalidState);
        }
        if let (Some(boundary), Some(preceding)) = (chat.incoming_key_cutover, preceding) {
            let is_preceding = fingerprint == preceding.fingerprint();
            if (raw >= boundary) == is_preceding {
                return Err(Error::InvalidState);
            }
        }
    }
    Ok(())
}
