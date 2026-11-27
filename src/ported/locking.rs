//! The Rust port of `rust/adapters/locking_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `check` (`rust/adapters/locking_adapter.py:11`).
pub fn check(
    generation: i64,
    current_generation: i64,
    stopping: bool,
    installed: bool,
) -> Result<serde_json::Value, String> {
    let refusal = "the secret-chat manager is stopping";
    Ok(
        serde_json::json!({"generation": (stopping || generation != current_generation).then_some(refusal), "current": (!installed).then_some(refusal)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_baseline_contract() -> Result<(), String> {
        let v = check(1, 2, false, true)?;
        assert!(v["generation"].is_string());
        assert!(v["current"].is_null());
        Ok(())
    }
}
