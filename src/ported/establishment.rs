//! The Rust port of `rust/adapters/establishment_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `remember` (`rust/adapters/establishment_adapter.py:9`).
pub fn remember(
    initial: i64,
    present: bool,
    discarded: bool,
    incoming_discarded: bool,
) -> Result<serde_json::Value, String> {
    let size = initial.clamp(0, 101) + i64::from(present);
    let updated = !(present && discarded) && (present || size < 100);
    Ok(serde_json::json!({
        "size": size + i64::from(!present && updated),
        "present": present || updated,
        "discarded": if updated { incoming_discarded } else { present && discarded },
        "updated": updated,
    }))
}
#[cfg(test)]
mod tests {
    use super::remember;
    #[test]
    fn terminal_has_precedence_and_capacity_only_blocks_new_entries() -> Result<(), String> {
        assert_eq!(remember(99, true, true, false)?["updated"], false);
        assert_eq!(remember(100, false, false, true)?["present"], false);
        assert_eq!(remember(100, true, false, true)?["discarded"], true);
        Ok(())
    }
}
