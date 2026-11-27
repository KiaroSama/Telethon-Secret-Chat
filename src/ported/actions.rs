//! The Rust port of `rust/adapters/actions_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `build` (`rust/adapters/actions_adapter.py:10`).
pub fn build(
    kind: String,
    first: i64,
    last: i64,
    ids: Vec<i64>,
) -> Result<Option<serde_json::Value>, String> {
    let id: u32 = match kind.as_str() {
        "ttl" => 0xa1733aec,
        "read" => 0x0c4f40be,
        "delete" => 0x65614304,
        "screenshot" => 0x8ac1f475,
        "flush" => 0x6719e45c,
        "typing" => 0xccb27641,
        "layer" => 0xf3048883,
        "resend" => 0x511110b0,
        _ => return Ok(None),
    };
    fn number(value: i64) -> Result<[u8; 4], String> {
        if let Ok(signed) = i32::try_from(value) {
            return Ok(signed.to_le_bytes());
        }
        u32::try_from(value)
            .map(u32::to_le_bytes)
            .map_err(|_| "error: 'I' format requires 0 <= number <= 4294967295".into())
    }
    let mut bytes = id.to_le_bytes().to_vec();
    match kind.as_str() {
        "ttl" | "layer" => bytes.extend(number(first)?),
        "resend" => {
            bytes.extend(number(first)?);
            bytes.extend(number(last)?);
        }
        "typing" => bytes.extend(0x16bf744eu32.to_le_bytes()),
        "read" | "delete" | "screenshot" => {
            bytes.extend(0x1cb5c415u32.to_le_bytes());
            bytes.extend(number(
                i64::try_from(ids.len()).map_err(|_| "Action list exceeds the supported length")?,
            )?);
            for random_id in ids {
                bytes.extend(random_id.to_le_bytes());
            }
        }
        _ => {}
    }
    let group = if matches!(kind.as_str(), "layer" | "resend") {
        "protocol"
    } else {
        "conversation"
    };
    Ok(Some(
        serde_json::json!({"bytes":{"$bytes":bytes},"group":group}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_baseline_contract() -> Result<(), String> {
        let v = build("resend".into(), 1, 5, vec![])?.ok_or("missing action")?;
        assert_eq!(v["group"], "protocol");
        assert_eq!(v["bytes"]["$bytes"].as_array().map(Vec::len), Some(12));
        Ok(())
    }
}
