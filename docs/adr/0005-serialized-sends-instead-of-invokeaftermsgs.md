# 0005. Serialized sends instead of an invokeAfterMsgs chain

- Status: accepted
- Date: 2026-09-28 (decided 2026-09-19/26 in the feature spec)

## Context

§3.4 asks clients to send outgoing messages in an `invokeAfterMsgs` chain so the server queues
them in order; a message overtaken by a later one opens a gap at the peer, and a gap that is
never filled ends the chat. A chain ties each request to the MTProto `msg_id` of the one before
it, which Telethon assigns inside its sender, and a chain does not survive a restart.

## Decision

Outgoing messages of one chat are serialized: each send holds the chat's lock, commits its
frame and sequence number, awaits its RPC, and only then lets the next one go. Before every
send, records whose transmission was not confirmed are retried first with their original
bytes and `random_id` (`outbox._send`, `outbox.retry_pending`), and again at `start()`.

## Consequences

- Order at the server follows commit order without depending on Telethon internals, across
  restarts too.
- Throughput per chat is one RPC round trip per message.
- A record that keeps failing stays pending and is retried before each later send; the
  records behind it still go, and a permanent rejection is withdrawn as a §3.8 self-delete
  (`outbox._rejected`), so one dead record cannot wedge a chat.
