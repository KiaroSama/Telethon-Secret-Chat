//! Convert the caller's visualization bytes into the official-client pixel order.

use serde_json::{Value, json};

pub fn key_visualization(data: Vec<u8>) -> Result<Value, String> {
    if data.len() != 36 {
        return Err("ValueError: a key visualization value is exactly 36 bytes".into());
    }
    let rows: Vec<Vec<u8>> = data
        .as_chunks::<3>()
        .0
        .iter()
        .map(|bytes| {
            bytes
                .iter()
                .flat_map(|byte| (0..4).map(move |pair| (byte >> (pair * 2)) & 3))
                .collect()
        })
        .collect();
    let hex: String = data[..32]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(json!({"rows":rows,"hex":hex}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_pair_order_and_row_boundaries() {
        let result = key_visualization(vec![228; 36]);
        assert_eq!(
            result.as_ref().map(|v| &v["rows"][0]),
            Ok(&json!([0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3]))
        );
        assert!(key_visualization(vec![0; 35]).is_err());
    }
}
