"""T042 / SC-001: a real chat with a real account, read by an official client.

Everything else in this repository proves the package agrees with itself or with
Telethon's own primitives. This file is the only one that proves the thing the
package is actually for: that Telegram's servers accept what it writes and an
official client decrypts it.

Three assertions, in the order the protocol produces them:

1. **Both ends computed the same key.** Not "this end computed a fingerprint" -
   ``ChatReady.key_fingerprint`` is only what this end derived. The far end's number
   arrives on the raw ``UpdateEncryption``, and the whole point of a fingerprint is
   the comparison. §3.4 of the design notes puts it plainly: a chat whose two ends
   hold different keys behaves exactly like a peer that stopped answering, so the
   comparison is the difference between a failure and a mystery.

2. **What this end writes, the far end reads.** A nonce in the text, echoed back by
   hand, is the only evidence that survives the round trip - the server sees
   ciphertext, so no amount of API inspection can stand in for a human reading it.

3. **What the far end writes, this end reads.** The reply decrypts here, with a
   sequence number the package assigned rather than one it was handed.

The chat is closed in ``finally``. An interop run that leaves chats behind fills the
operator's account with them, and the next run's "accept the new secret chat" becomes
an instruction they cannot follow.
"""

import secrets

import pytest

from ._live import HUMAN_DEADLINE, await_event, ready_chat

# Above ``HUMAN_DEADLINE`` plus the server steps around it. pyproject sets a global
# 120s timeout, which is right for a tier that never waits for a person and would
# kill this one while the operator is still unlocking a phone.
pytestmark = pytest.mark.timeout(600)


async def test_both_ends_agree_on_the_key_fingerprint(manager, peer, announce):
    chat, ready = await ready_chat(manager, peer, announce, "fingerprint check")
    try:
        mine = ready.key_fingerprint
        theirs = manager.recorder.peer_fingerprints.get(chat.id)

        assert mine, "this end reported a zero fingerprint, which is not a key"
        assert theirs is not None, (
            "no encryptedChat update carried the peer's fingerprint - the chat "
            "reached ready without the number the comparison is made against"
        )
        assert (
            mine == theirs
        ), f"the two ends derived different keys: this end {mine}, the peer {theirs}"
    finally:
        await manager.close(chat.id, reason="interop fingerprint check finished")


async def test_a_message_crosses_in_both_directions(manager, peer, announce):
    chat, _ = await ready_chat(manager, peer, announce, "message round trip")
    nonce = secrets.token_hex(4)
    try:
        await manager.send_message(chat.id, f"interop {nonce}")

        announce(
            f"On the second account, read the message and REPLY with the same code: "
            f"{nonce}. Waiting up to {HUMAN_DEADLINE:.0f}s."
        )
        received = await await_event(
            manager.recorder,
            "MessageReceived",
            where=lambda one: one.chat_id == chat.id and nonce in (one.text or ""),
            deadline=HUMAN_DEADLINE,
            missing=(
                f"no reply carrying {nonce} arrived - either the official client "
                "could not read what this package sent, or the reply did not decrypt"
            ),
        )

        assert received.seq_no >= 0, "a decrypted message with no sequence number"
        assert received.random_id, "a decrypted message with no random_id"

        # The history the manager keeps is what an application reads; a message that
        # fired an event and is absent from it is only half delivered.
        assert any(
            one.random_id == received.random_id for one in manager.read_history(chat.id)
        ), "the received message never reached the readable history"
    finally:
        await manager.close(chat.id, reason="interop round trip finished")


async def test_closing_here_is_seen_as_closed_here(manager, peer, announce):
    """A chat closed by this end stops being sendable, whatever the far end does.

    Deliberately not an assertion about the official client's screen: the far end
    learns of the closure through a service message, and how a given client version
    renders that is not this package's contract. What IS its contract is that the
    chat becomes unsendable here the moment it is closed, so a later send cannot
    encrypt under a key the application believes is gone.
    """
    chat, _ = await ready_chat(manager, peer, announce, "closure check")
    await manager.close(chat.id, reason="interop closure check")

    from telethon_secret_chat import ChatClosed

    with pytest.raises(ChatClosed):
        await manager.send_message(chat.id, "this must not encrypt")
