//! Dependency-free rendering of an already derived Telegram key hash.
pub const PALETTE: [&str; 4] = ["#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9"];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyVisualization {
    pub rows: [[u8; 12]; 12],
    pub hex: String,
}
pub fn render(hash: &[u8]) -> Option<KeyVisualization> {
    if hash.len() != 36 {
        return None;
    }
    let mut rows = [[0; 12]; 12];
    for n in 0..144 {
        rows[n / 12][n % 12] = (hash[n / 4] >> (2 * (n % 4))) & 3;
    }
    use std::fmt::Write;
    let mut hex = String::with_capacity(64);
    for byte in &hash[..32] {
        write!(&mut hex, "{byte:02x}").ok()?;
    }
    Some(KeyVisualization { rows, hex })
}
