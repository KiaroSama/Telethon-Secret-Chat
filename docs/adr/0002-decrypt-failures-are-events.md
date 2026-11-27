# 0002. A failed decrypt is an event, and a refusal carries a shape

- Status: accepted
- Date: 2026-09-28 (decided 2026-09-19 in the feature spec)

## Context

§2.7 and §3.4-§3.6 say a bad message must be rejected, and some violations must abort the
chat. Rejection happens inside Telethon's update dispatch: an exception raised there reaches
the application as a traceback from a library it did not call. And §2.7 warns that reporting
which check failed at which offset rebuilds a decryption oracle.

## Decision

The update handler (`receive._on_update`) turns every refusal into a `DecryptFailed(chat_id,
reason)` event. Which failures end the chat is fixed by the protocol: wrong parity, an echo
that goes backwards or claims an unsent message, a layer that goes down, or a gap beyond the
1000-message window abort it (`sequence.preflight`); a replay is dropped silently; a gap is
queued and one resend is requested (`sequence.accept`). A chat that ends also emits
`ChatClosedEvent`. `reason` is a phrase this package wrote; no error or event is ever
formatted from a key, a plaintext, a ciphertext or a wire object (`errors.py`).

## Consequences

- An application's update loop never sees an exception from this package's receive path.
- An application that ignores `DecryptFailed` loses nothing it could have acted on: the
  message was refused either way.
- Diagnosis relies on the reason phrases and the chat id, never on the rejected bytes.
