//! The Rust port of `rust/adapters/async_seam_proof_adapter.py`.
//!
//! Replace each `unimplemented!` with the port of the source function. Return `Err` where
//! the source throws or raises: the caller gets an error there too. After changing a
//! signature, run `rust-anything refresh` to regenerate the bindings. `rust-anything diff` checks
//! behavior against what the source recorded.

/// Port of `close_boundary` (`rust/adapters/async_seam_proof_adapter.py:27`).
pub fn close_boundary(outcome: i64) -> Result<serde_json::Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| "Could not start isolated test runtime")?;
    runtime.block_on(async {
        let (entered, entry) = tokio::sync::oneshot::channel();
        let mut task = tokio::spawn(async move {
            let reason = "stored chat lacks its DH parameters and cannot rekey";
            entered
                .send((1, reason))
                .map_err(|_| "Offline close observer went away")?;
            match outcome {
                1 => Err("synthetic close failure"),
                2 => std::future::pending::<Result<(), &str>>().await,
                _ => Ok(()),
            }
        });
        let call = tokio::time::timeout(std::time::Duration::from_secs(1), entry)
            .await
            .map_err(|_| "Timed out waiting for offline close")?
            .map_err(|_| "Offline close was not reached")?;
        if outcome == 2 {
            task.abort();
        }
        let status = match tokio::time::timeout(std::time::Duration::from_secs(1), &mut task).await
        {
            Ok(Ok(Ok(()))) => "ok",
            Ok(Ok(Err(_))) => "error",
            Ok(Err(error)) if error.is_cancelled() => "cancelled",
            _ => {
                task.abort();
                return Err("Offline close did not settle".into());
            }
        };
        Ok(serde_json::json!({"calls":[call],"status":status,"exchange_started":false}))
    })
}
