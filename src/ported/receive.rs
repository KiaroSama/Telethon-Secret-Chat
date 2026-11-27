//! The Rust port of `rust/adapters/receive_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `history` (`rust/adapters/receive_adapter.py:9`).
use std::collections::{BTreeSet, HashSet};

pub fn history(
    ids: Vec<i64>,
    remove: Vec<i64>,
    limit: i64,
    forget: bool,
) -> Result<serde_json::Value, String> {
    if forget {
        return Ok(serde_json::json!({"retained":[], "indexed":[], "present":false}));
    }
    let remove: HashSet<_> = remove.into_iter().collect();
    let kept: Vec<_> = ids.into_iter().filter(|id| !remove.contains(id)).collect();
    // Python rebuilds the index from the filtered input before the deque truncates it.
    let indexed: BTreeSet<_> = kept.iter().copied().collect();
    let start = kept.len().saturating_sub(limit.clamp(0, 256) as usize);
    Ok(serde_json::json!({"retained": &kept[start..], "indexed":indexed, "present":true}))
}
#[cfg(test)]
mod tests {
    #[test]
    fn preserves_order_and_indexes_pre_truncation_history() -> Result<(), String> {
        let result = super::history(vec![3, 1, 2, 1, 4], vec![3], 2, false)?;
        assert_eq!(result["retained"], serde_json::json!([1, 4]));
        assert_eq!(result["indexed"], serde_json::json!([1, 2, 4]));
        assert_eq!(super::history(vec![1], vec![], 1, true)?["present"], false);
        Ok(())
    }
}
