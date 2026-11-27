# 0004. Rekey automatically on the documented trigger, with a two-key window

- Status: accepted
- Date: 2026-09-28 (decided 2026-09-19 in the feature spec)

## Context

Official clients require forward secrecy in secret chats (§4). The trigger (100 messages or a
week, §4.1) is documented as client behaviour, not a rule. During an exchange each side keeps
encrypting with the old key until its own commit point, so a message still in flight under
the old key must stay readable (§4.8); the archived package overwrote the key outright.

## Decision

The package starts a rekey itself when `rekey.should_rekey` says the trigger is due, checked
before every send, service actions included; `rekey(chat_id)` starts one on demand. Sending
stays allowed while `rekeying`. A receiver holds up to three keys - current, pending, previous -
and picks by the frame's fingerprint (`rekey.select_key`). The previous key is kept until the
new one is confirmed by a message from the peer AND no gap is open, then discarded
(`rekey.retire_previous_key_if_settled`). Sequence counters are never reset by a rekey.

## Consequences

- An application needs to do nothing for forward secrecy.
- A chat with an open gap keeps its previous key longer; that key is still scrubbed on close.
- The key visualization is drawn from the FIRST key and never changes, as official clients do.
