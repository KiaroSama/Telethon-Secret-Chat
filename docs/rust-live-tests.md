# Explicit live interoperability tier

Ten native cases in `tests/rust_live_interop.rs` compile in normal CI and remain
ignored. They are real grammers network applications when explicitly invoked,
not synthetic transports. Compilation/collection never opens a session document.
No real account run is claimed by the offline rewrite evidence.

## Inputs and scope

Use dedicated, idle, already-authorized test accounts and an official Telegram
client on the peer side. Never share their authorization with another active
SDK process during a run. The harness has no login/OTP/session-import flow and
does not create authorization keys. It constructs the test caller's original
in-memory SDK session from the explicitly supplied existing test key.

`TSC_LIVE_SESSION_A` points to a small owner-only JSON file outside the repository:

```json
{
  "api_id": 123,
  "user_id": 1000,
  "dc_id": 2,
  "auth_key": "512 hexadecimal characters from the dedicated existing test authorization",
  "peer_user_id": 2000,
  "peer_access_hash": 123456
}
```

The placeholder is not a key. Supply real dedicated values privately. Unix mode
must exclude group/other access; symlinks and oversized/non-file inputs refuse.
The parser redacts failures and zeroizes its raw input/key string. Windows users
must apply owner-only ACLs. Do not commit, print or upload authorization documents.

For paired cases, `TSC_LIVE_SESSION_B` describes a second dedicated existing
account with reversed user/peer identities and its own access hash/key. Both
server-confirmed self users must match their configuration and be non-bots.
Each case owns a new temporary protocol store, only its newly created secret chats
and the original receiver/runner it started. Updates for unrelated peers are not
applied by this harness. This tier does not certify strong ingress custody.

## Invocation

Set `TSC_RUN_LIVE=1` and the explicit session path(s), then select one case:

```sh
cargo test --locked --test rust_live_interop message_crosses_both_directions -- --ignored --exact --nocapture --test-threads=1
```

Setting the environment flag alone does not run ignored tests. The harness also
serializes cases internally. Do not run the full ignored suite unless prepared
for every listed real effect, including deleting the throwaway chat on both sides.

The official-client cases require accepting the new chat, reading/replying to a
random displayed nonce, opening media and confirming readable output. Send an
actual file for the incoming-file case. `TSC_TEST_MEDIA_DIR` can supply real media
fixtures for additional kinds; generated PNG/JPEG/document fixtures need no codec
process. `TSC_TEST_NONCE` optionally supplies a short ASCII test marker; otherwise
it is random. Prompts print test instructions/markers, not authorization material.

Paired cases exercise ordered traffic both ways, PFS key agreement, streamed
small/multipart files, restart on the same client/receiver, peer-close update sync,
terminal send refusal and delete-for-both. Receiver ownership is transferred back
during restart rather than opening another stream or session.

Setup, scenario and cleanup have deadlines. Cleanup attempts closing each newly
created chat, settling the manager, pausing the original receiver and quitting/
joining the runner. A local close result does not independently prove a remote
RPC completed. Interruption or cleanup refusal prints a fixed message: inspect
the throwaway chats in the official clients before another run. Private temporary
state cleanup cannot erase every OS/allocator copy or retract queued remote work.
