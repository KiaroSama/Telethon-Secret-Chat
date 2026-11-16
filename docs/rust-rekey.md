# Native rekey state and delivery

`migration/rekey-mappings.json` preserves the 38 remaining collected cases from the original `test_rekey.py`. The file originally collected 40 cases: the automatic trigger and concurrent service-send trigger cases are already preserved by the manager ledger. They are excluded from this ledger to avoid duplication.

The 24 core cases include nineteen ordinary unit cases and five scenarios whose original asynchronous handlers now use Rust's synchronous state/effect engine inside asynchronous tests. The fourteen remaining cases exercise real native managers, temporary-file restart and concurrent update tasks through fake transports. All cryptographic operations use native primitives and real ephemeral key material; no account or external network is involved.

`rekey::should_rekey` retains the strict more-than-100-message and more-than-one-week conditions, the used-key requirement and guards against an active exchange or retained previous key. Exchange IDs use the full signed 64-bit OS-backed producer also used by the actual RequestKey path. Collision comparisons are signed. `may_abort` requires an active exchange and rejects accepted/committed phases.

`select_key` is the actual protocol receive selector: current, pending accepted and previous keys are considered by their fingerprints. `adopt_new_key` is the same engine used by CommitKey and first-new-key-packet handling. It retains the previous key, clears exchange state, resets rekey accounting and preserves sequence counters and the initial visual hash. Previous key retirement requires new-key confirmation and no remaining gap.

The manager cases verify that pending service steps survive local and peer history flushes, messages can arrive during an exchange, both peers converge after crossing requests, and concurrent tasks preserve that convergence. The file-backed case stops and reopens an actual exchange while the peer's reply is queued, checks the retained current key and exchange identity, then completes the exchange through the revived manager.

Authenticated unsafe RequestKey and mismatched CommitKey fingerprints are fatal protocol refusals. The tests first observed those chats incorrectly remaining Ready/Rekeying, then verified the narrow manager fix: one close event, discarded pending mailbox work, scrubbed key state and the typed fixed `CloseReason::RekeyFingerprint` diagnostic. Ordinary caller-facing file fingerprint validation uses its separate nonfatal path. A mismatched AcceptKey remains a recoverable aborted exchange, preserving the old usable key.
