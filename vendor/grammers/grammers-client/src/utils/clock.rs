// Copyright 2026 - Kiaro native SDK supplement. MIT OR Apache-2.0.
use std::time::SystemTime;

/// Seed non-secret, unique request identifiers without trusting the wall clock.
pub(crate) fn random_id_seed(time: SystemTime) -> i64 {
    let seed = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or(1);
    if seed == 0 { 1 } else { seed }
}
