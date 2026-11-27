//! The Rust port of `rust/adapters/outbox_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `retained` (`rust/adapters/outbox_adapter.py:8`).
use std::collections::BTreeSet;

pub fn retained(ids: Vec<i64>, services: Vec<bool>) -> Result<Vec<i64>, String> {
    Ok(ids
        .into_iter()
        .zip(services)
        .filter_map(|(id, service)| (!service).then_some(id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}
#[cfg(test)]
mod tests {
    #[test]
    fn filters_service_and_deduplicates_content() -> Result<(), String> {
        assert_eq!(
            super::retained(vec![4, -1, 4, 2], vec![false, true, false, false])?,
            [2, 4]
        );
        Ok(())
    }
}
