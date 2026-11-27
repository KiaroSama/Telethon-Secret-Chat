//! The Rust port of `rust/adapters/rekey_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

use serde_json::json;
use sha1::{Digest, Sha1};

fn exchange_id() -> Result<i64, String> {
    let mut raw = [0; 8];
    getrandom::fill(&mut raw).map_err(|_| "RuntimeError: OS randomness unavailable")?;
    Ok(i64::from_le_bytes(raw))
}

/// Port of `collision` (`rust/adapters/rekey_adapter.py:12`).
pub fn collision(mine: i64, theirs: i64) -> Result<String, String> {
    use std::cmp::Ordering;
    Ok(match mine.cmp(&theirs) {
        Ordering::Greater => "abandon_theirs",
        Ordering::Less => "join_theirs",
        Ordering::Equal => "abort_both",
    }
    .into())
}

/// Port of `exchange_id_contract` (`rust/adapters/rekey_adapter.py:16`).
pub fn exchange_id_contract() -> Result<bool, String> {
    let _id = exchange_id()?;
    Ok(true)
}

/// Port of `decisions` (`rust/adapters/rekey_adapter.py:21`).
///
/// `role` defaults to `""` in the source.
pub fn decisions(
    messages: i64,
    age: i64,
    exchange: bool,
    old_key: bool,
    confirmed: bool,
    gap: bool,
    role: String,
) -> Result<serde_json::Value, String> {
    let due = !exchange && !old_key && messages > 0 && (messages > 100 || age > 7 * 24 * 60 * 60);
    let may_abort = exchange && !matches!(role.as_str(), "accepted" | "committed");
    Ok(
        json!({"due": due, "may_abort": may_abort, "keeps_previous": old_key && (!confirmed || gap)}),
    )
}

/// Port of `key_state` (`rust/adapters/rekey_adapter.py:36`).
pub fn key_state(
    command: String,
    tag: i64,
    slot: i64,
) -> Result<Option<serde_json::Value>, String> {
    #[derive(Default)]
    struct KeyState {
        active: u8,
        previous: Option<u8>,
        pending: Option<u8>,
        confirmed: bool,
        messages: usize,
        exchange: Option<i64>,
        rekeying: bool,
    }
    let mut state = KeyState {
        previous: Some(2),
        pending: Some(1),
        ..KeyState::default()
    };
    match command.as_str() {
        "select" => {
            let wanted = u8::try_from(slot.rem_euclid(4)).map_err(|_| "Invalid synthetic slot")?;
            let fingerprint = |tag| Sha1::digest([tag; 256]);
            let target = fingerprint(wanted);
            let selected = [Some(state.active), state.pending, state.previous]
                .into_iter()
                .flatten()
                .find(|&id| fingerprint(id)[12..] == target[12..]);
            return Ok(Some(json!({"selected": selected})));
        }
        "adopt" => {
            state.previous = Some(state.active);
            state.active =
                u8::try_from(tag.rem_euclid(256)).map_err(|_| "Invalid synthetic tag")?;
            state.pending = None;
            state.exchange = None;
            state.confirmed = false;
            state.messages = 0;
        }
        "clear" => {
            state.pending = None;
            state.exchange = None;
            state.rekeying = false;
        }
        "retire" => state.previous = None,
        _ => return Ok(None),
    }
    Ok(Some(
        json!({"has_previous": state.previous.is_some(), "has_pending": state.pending.is_some(), "confirmed": state.confirmed, "messages": state.messages, "state": if state.rekeying {"rekeying"} else {"ready"}, "exchange": state.exchange.is_some(), "active_tag": state.active}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_and_commit_guards_match() -> Result<(), String> {
        assert_eq!(collision(2, 2)?, "abort_both");
        assert_eq!(collision(3, 2)?, "abandon_theirs");
        assert_eq!(collision(1, 2)?, "join_theirs");
        let kept = decisions(101, 604801, true, true, true, true, "committed".into())?;
        assert_eq!(
            kept,
            serde_json::json!({"due":false,"may_abort":false,"keeps_previous":true})
        );
        assert!(exchange_id_contract()?);
        Ok(())
    }
}
