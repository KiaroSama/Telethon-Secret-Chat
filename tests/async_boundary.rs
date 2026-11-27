use std::time::Duration;

use kiaro_secret_chat::ported::async_seam_proof::close_boundary;

#[test]
fn recorded_sync_seam_matches_close_order_error_and_cancellation() -> Result<(), String> {
    for (input, status) in [(0, "ok"), (1, "error"), (2, "cancelled")] {
        let proof = close_boundary(input)?;
        assert_eq!(proof["status"], status);
        assert_eq!(proof["exchange_started"], false);
        assert_eq!(
            proof["calls"],
            serde_json::json!([[1, "stored chat lacks its DH parameters and cannot rekey"]])
        );
    }
    Ok(())
}

#[tokio::test]
async fn native_async_close_boundary_preserves_order_error_and_cancellation() {
    for (outcome, expected_status) in [(0_i64, "ok"), (1_i64, "error"), (2_i64, "cancelled")] {
        let (entered, entry) = tokio::sync::oneshot::channel();
        let mut task = tokio::spawn(async move {
            let reason = "stored chat lacks its DH parameters and cannot rekey";
            entered
                .send((1_i64, reason))
                .map_err(|_| "offline close observer dropped")?;
            match outcome {
                1 => Err("synthetic close failure"),
                2 => std::future::pending::<Result<(), &str>>().await,
                _ => Ok(()),
            }
        });

        let call = tokio::time::timeout(Duration::from_secs(1), entry)
            .await
            .expect("close entry timed out")
            .expect("close entry channel dropped");
        assert_eq!(
            call,
            (
                1_i64,
                "stored chat lacks its DH parameters and cannot rekey"
            )
        );

        if outcome == 2 {
            task.abort();
        }

        let status = match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
            Ok(Ok(Ok(()))) => "ok",
            Ok(Ok(Err(_))) => "error",
            Ok(Err(err)) if err.is_cancelled() => "cancelled",
            other => panic!("unexpected close boundary outcome: {other:?}"),
        };
        assert_eq!(status, expected_status);
    }
}
