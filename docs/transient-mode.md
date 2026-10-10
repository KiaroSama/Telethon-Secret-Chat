# Transient payload mode

**Library CI validated; consumer integration not exercised.** The implementation at `685181b4ed1519f3bc77638454075ddeeddeb0be` passed the full Linux/Windows matrix and installed-wheel checks. It is available for an explicitly authorized consumer integration under the limitations below, not a claim that a consumer or live Telegram deployment was tested. No new release/tag has been issued.

Use `TransientSecretChatManager` explicitly, not `history_limit=0` on the default manager. The default manager remains unchanged and persists its protocol mailbox, gap and original-wire outbox. This opt-in mode uses the caller's exact existing connected Telethon client and authorization, without creating, connecting or disconnecting another client, copying a session, logging in, or changing device settings.

## Minimal same-existing-client example

```python
from telethon_secret_chat import ProtectedFileStorage, TransientSecretChatManager

async def attach(existing_client, protected_path, visible_handler):
    # The caller already owns authorization, connection, directory protection and UI.
    manager = TransientSecretChatManager(
        existing_client, ProtectedFileStorage(protected_path)
    )

    async def message_received(event):
        await visible_handler(event)  # must not log or archive ordinary content
        # No automatic read acknowledgement or TTL countdown.

    manager.on("MessageReceived", message_received)
    await manager.start()
    return manager

# Stop accepting/creating new chats while servicing established chats in this run:
# manager.set_accepting_requests(False)
# Explicit read, only after the application's chosen user-visible read/open event:
# await manager.mark_read(chat_id, [random_id])
# Shutdown detaches and permanently suspends known chats; does not discard them:
# await manager.stop()
```

No forum destination, relay, automatic acceptance, read policy or account activation is supplied. A future cloud/forum copy is cloud storage, **not E2E Secret Chat storage**.

## Public surface

- `TransientSecretChatManager(client, storage: ProtectedFileStorage, *, limits=TransientLimits())`.
- `await start()`, `await stop()`, `await settle()`; async callbacks through existing `on(event, handler)`. Sync callbacks are refused because they cannot be bounded cooperatively. An ordinary message arriving without a MessageReceived handler suspends the chat instead of silently claiming delivery.
- `set_accepting_requests(enabled: bool)`; `is_suspended(chat_id) -> bool`; existing `list()`/`status()` return safe snapshots.
- Existing bounded `create(user)`, `accept(chat_id)`, `send_message(chat_id, text, entities=None, reply_to=None)`, `rekey(chat_id)`, `retry_pending(chat_id)`, `set_ttl(chat_id, seconds)`, `mark_read(chat_id, random_ids)`, `screenshot(chat_id, random_ids)`, `set_typing(chat_id, action=None)`, `forward_file(chat_id, source, *, caption="", reply_to=None)`.
- `send_file(chat_id, source, **metadata)` reuses streamed existing encryption and refuses oversized input. `receive_file(message) -> bytes` validates/decrypts into bounded RAM; no disk intermediate.

`save_file`, auto-save, saved-copy/export, delete, forget and flush APIs are refused before their persistence/RPC paths. `close` in this opt-in manager means permanent local suspension, **not** the default manager's remote close. No resume API exists.

`TransientRefused` carries a content-free failure. `TransientCleanupIncomplete` means owned work remains: **shutdown is not complete**, and restart is refused. Never catch it and claim cleanup succeeded. Raw transport/media exceptions are not chained to these public failures.

## Protected-state schema v1

A new `ProtectedFileStorage(path)` contains exactly `chats`, empty `out`, empty `in`, and `mode="protected-v1"`. Existing default-mode files, even empty ones, are refused without migration. Windows caller must preconfigure an owner-only directory ACL; POSIX file600/directory700 protection is reused. This is one-process storage, not at-rest encryption or an inter-process ownership lock.

Each `chats` entry has exactly:

- `format=1`;
- `binding={user_id, auth_key_id, dc_id}` from the caller's connected account/session (no raw authorization key/session export);
- `suspended: bool`;
- `chat` with the existing validated fields below, and **no** `pending_deliveries`.

```text
id access_hash peer_user_id is_outbound state
key key_fingerprint pending_key previous_key
exchange_id exchange_secret rekey_role dh_prime dh_g
in_seq_no out_seq_no peer_in_seq_no
 gap_requested resend_due gap_end new_key_confirmed
layer wrapper_layer ttl created_at rekeyed_at messages_since_rekey
admin_id participant_id initial_key_hash handshake closed_reason
```

`closed_reason` must be null. `handshake` accepts only validated `p/g/g_a/secret` DH integers. Key lengths, fingerprints, states, counters and ranges are validated; unexpected fields are rejected, not silently dropped. Store maximum4MiB, individual protected envelope64KiB, at most64chats. No ordinary or service wrapper/body/frame/file reference/key enters durable queues, journals, snapshots or settings. **Durable service retransmission payload limit: zero.** All service frames also remain volatile.

Transactions coordinate complete RAM rollback with the existing synchronous atomic file replacement. No network await is inside a transaction. The internal projection is explicitly part of this mode; this is not an ordinary backend pretending to save full records while dropping them.

## Budgets

`TransientLimits` is immutable and validates positive finite types; booleans, infinity and values above ceilings refuse.

| Field | Default | Ceiling |
|---|---:|---:|
| chats | 64 | 64 |
| records | 512 | 4096 |
| bytes | 8MiB | 64MiB |
| frame_bytes | 256KiB | 1MiB |
| retention_seconds | 300 | 3600 |
| operation_seconds | 15 | 60 |
| cleanup_seconds | 5 | 30 |
| tasks | 8 | 64 |
| attempts | 3 | 10 |

Records count global retained outbox, gaps and delivery mailbox. Bytes charge strings/bytes/containers, active inputs and callback events; an additional4x state reservation accommodates rollback/serialization copies. This is a finite **content accounting budget, not a process-RSS guarantee**. Callers, Telethon's own update queues/serialization/retries, interpreter allocation, OS swap/crash dumps and external logs are outside library control. Returned objects/bytes become the consumer's responsibility. The library cannot guarantee RAM erasure.

Expired or missing required frames cause suspension, never silent eviction followed by continued protocol. Attempts cap each original retained frame. Uncertain/permanent transport failure suspends rather than making the default durable `SendPending` promise. Each owned operation has one deadline coordinator rather than competing cancellation timers; direct callbacks retain their own cooperative deadline. Cancellation-resistant client/consumer code can prevent true settlement, which is reported as incomplete cleanup instead of hidden success. No cooperative Python library can forcibly terminate an arbitrary coroutine/thread without owning its process.

## Restart, gaps and PFS limitations

- Every stop and every reconstructed stored chat is permanently suspended, including pending handshakes. Same-manager restart does **not** resume those chats. Disabling only new acceptance is different: live established chats continue in the same running manager.
- While live and within budgets, existing authenticated sequence/parity/duplicate/gap/original-byte resend/PFS code is reused. A missing ordinary outgoing slot cannot be rebuilt from keys or replaying only services. Never reset counters, reuse a lost slot, or fabricate Noop/Delete replacements.
- Suspension preserves protected material and refuses further protocol transitions/sends/delivery, without automatic remote discard/delete. The peer may terminate independently.
- Telegram normally requires abort for an unsatisfiable Resend. Local suspension is **not full protocol recovery/abort compliance**; it is refusal to continue. Preserved previous keys in unusable suspended chats are also **not normal PFS retirement/secure-erasure compliance**. Disposal is a separate explicit caller decision.
- Receipt into RAM, consumer callback completion, server send success, peer authenticated sequence acknowledgement, transport qts and user read are separate facts. RAM delivery is lossy across crashes and no replay/at-least-once consumer guarantee exists.
- The caller's Telethon client may advance and acknowledge qts before the addon processes a callback. This mode does not intercept qts or promise to retain server ciphertext. Stopping acceptance/handlers cannot undo cumulative transport acknowledgement.
- `mark_read` and `set_ttl` remain explicit. Receipt, callbacks, startup and shutdown do not send read receipts or start a local lifetime timer.

## Evidence and pins

Initial CI-tested implementation pin: `685181b4ed1519f3bc77638454075ddeeddeb0be`. A later CI run exposed a cancellation-ordering race; the single-owner deadline correction is included in subsequent commits. Use the final merged commit supplied in the handoff, not this initial pin, when integrating. Distribution metadata remains `kiaro-telethon-secret-chat==0.2.0` (alpha), Python>=3.11, Telethon>=1.45,<2. The existing `v0.2.0` tag predates these APIs: pin this commit rather than the version/tag alone. Dependencies remain those in its `uv.lock`; no additional runtime dependency was introduced.

[Tests and matrix agreement](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/runs/38087743007), [installed wheel](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/runs/38087742901), [lint/format/types](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/runs/38087742902) and [CodeQL](https://github.com/KiaroSama/Telethon-Secret-Chat/actions/runs/38087743017) completed successfully on that exact SHA. All six matrix legs agreed on 1744 test identities; Linux ran1726 with18 declared skips and91.43% coverage. Live Telegram cases were intentionally not executed.

Local guarded synthetic checks observed: strict store refusal, same-client ordinary send/receive, callback failure suspension, memory-only media receipt, reconstructed-chat refusal with retained keys, synthetic rekey, policy-off continuation, cancellation-resistant network cleanup refusal, all atomic snapshots/logs/backup sentinel absence, suspension-write-fault cancellation, gap drain and duplicate rejection. These are separate development checks, not a final integrated acceptance pass. Tests use actual file and protocol seams with synthetic transport; they do **not** prove live official-client interoperability. Existing vector/official-client fixtures remain the crypto oracle. The complete discovered suite passed in the six-leg CI matrix, including the new synthetic acceptance cases and unchanged default-mode regressions; installed-wheel imports passed on Python3.11/3.13. No live operation or consumer integration was performed or authorized. Synthetic phase-restoration tests are not real process-kill/network interoperability evidence.

Sources: [Telegram e2e](https://core.telegram.org/api/end-to-end), [seq_no](https://core.telegram.org/api/end-to-end/seq_no), [PFS](https://core.telegram.org/api/end-to-end/pfs), [Telethon1.45 client](https://docs.telethon.dev/en/stable/modules/client.html). Current upstream references informed the refusal limits; no cryptography was rewritten.
