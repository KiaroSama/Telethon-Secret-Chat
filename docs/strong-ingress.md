# Strong ingress: development status and prerequisite contract

**Final release validation is pending.** Public `DurableIngress.prepare()` now
validates the exact patched prerequisite, protected store and explicit baseline
before attaching to an unconnected caller-owned client. Incompatible, live or
unbound configurations refuse. Do not infer live interoperability or consumer
installation approval from the synthetic checks described below.

## Why the prerequisite changes

Official Telethon 1.45.0 advances MessageBox state before Raw callbacks. A subsequent
`updates.getDifference` uses its input qts as an implicit queue acknowledgment;
withholding `messages.receivedQueue` alone is insufficient. The library's existing
bounded mode protects committed post-Raw custody, not this earlier gap.

The project-owned prerequisite tracks patches and helper source, not the full
upstream tree. `tools/build_telethon.py` downloads a checksum-pinned official wheel,
checks every edited source module, applies exact hunks and creates a normal
installable wheel with rewritten RECORD and provenance. See
[prerequisite instructions](../prerequisites/telethon/README.md).
No installed-package mutation, runtime monkeypatch, new client or authorization is
part of this approach. Upstream MIT notices remain in the resulting wheel.

## Separate boundaries

1. Durable ingress receipt commits an exact live envelope or complete difference
   response and its checkpoint/cursor before permitting queue acknowledgment.
2. Secret-protocol application commits counters/mailbox and applied cursor together.
3. Relay projection preserves consumer obligations until explicit acknowledgment.
4. External cloud-send settlement remains the consumer's responsibility.

Packet MTProto ACK, queue qts acknowledgment, peer sequence acknowledgment and a
user read receipt are different operations. Nothing here enables automatic read,
acceptance, device settings, session migration, forum/UI relay or account activation.

## Development APIs

- `EncryptedIngressStorage(path, *, key, binding, max_bytes)` supplies AEAD-encrypted
  transactional custody and exclusive file ownership. The caller supplies a
  32-byte key and binding fields `user_id`, `auth_key_id`, `dc_id`, `generation`.
  The caller manages keys separately and configures owner-only Windows ACLs.
- `IngressLimits(records, bytes, age_seconds, batch_bytes=1048576,
  operation_seconds=15)` specifies finite aggregate ingress budgets.
- `DurableIngress(storage, *, limits)` owns durable checkpoint and batch custody.
  Its bootstrap is explicit; it cannot recover history never held in the store.
- `StrongSecretChatManager(client, ingress, *, limits, admission=None)` reuses
  bounded protocol, retained send identities, relay polling and RAM media receipt.
  Auto-save is disabled before startup; previously saved archives remain untouched.

Call `await ingress.prepare(client, baseline=verified_state)` before connection
for a fresh store. `verified_state` is the caller's explicitly verified MessageBox
session-state tuple, not a guessed zero or recovery of already acknowledged history.
For a reconstructed store call `await ingress.prepare(client)` with no replacement
baseline. The coordinator verifies installed module hashes/import paths and restores
its protected checkpoint before startup state requests can skip owed work. Then
construct `StrongSecretChatManager(client, ingress, limits=custody_limits)` and start
it with the original caller-owned client. No connect/login is performed by prepare.

Use the same caller-key/binding/generation on reconstruction, and close the previous
manager and encrypted store before replacing them. Shutdown fences acknowledgments,
cancels/joins owned replay and retains ownership if cancellation-resistant code
cannot be joined. Asynchronous application callbacks cannot be forcibly killed;
external effects are not covered by a Python hard-deadline guarantee.

## Storage and operational limits

The encrypted file contains temporary protocol/ingress/relay custody, including
payloads and media keys where needed for recovery; it is not payload-free. No media
archive is created. Exclude custody and encryption keys from logs, exports,
backups, packages and deployment bundles. AEAD cannot prevent replacement with an
older valid snapshot without a separate trusted anti-rollback anchor.

POSIX file and directory fsync are required. Windows file fsync protects file data;
this is not a universal power-loss/hardware guarantee. On uncertainty the store
refuses further work; no physical-erasure guarantee is made.

qts is authorization-wide. Finite durable spooling can bridge a paused chat only
until aggregate capacity is exhausted; then authorization acknowledgment must
stop, preserving already committed custody. Unlimited per-chat isolation with
unaffected global acknowledgment and finite storage is not promised. Resume
requires verified continuity, not fabricated counters or a reset baseline.

Telegram can delete acknowledged or older-than-seven-day events and clears the
authorization queue on deauthorization. No local change removes those limits.

## Evidence and unfinished gates

Observed development checks include the original real-release gap reproduction,
pinned build hash refusal, AEAD checkpoint reconstruction/exclusive ownership,
whole qts-less difference custody, and an original PFS response retained despite
uncertain transmission. The PFS seam also checks an ordinary relay item preceding
that service action. These are non-sensitive deterministic fixtures, not owner
accounts or live Telegram interoperability.

Remaining release gates are independent final review and the complete Linux/Windows
fault, mixed-stream, backpressure and installed prerequisite/library wheel matrix
on the exact approved commit pair. CI builds and installs the prerequisite after
normal lockfile synchronization and then uses `--no-sync` to avoid silently restoring
official Telethon. Ordinary/default/transient regressions remain required.

Both explicit `receivedQueue(max_qts)` and implicit `getDifference(qts)` use a
verified shared durable frontier. Source tests exercise real release update-loop
and RPC handling with a server model that deletes acknowledged events. This model
proves only the tested local ordering and recovery boundaries, not a live Telegram
service run, indefinite offline recovery or exactly-once external cloud sends.
