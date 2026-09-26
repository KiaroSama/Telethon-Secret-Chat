"""Spec 003 / US1: a rekey this package starts, accepted by an official client.

Every other rekey test in this repository runs this package against itself, which
Principle I says proves nothing: two copies of one wrong CommitKey agree perfectly. Here
the acceptor is the operator's official client, so the four checks below are the first
evidence that the exchange - and the audit's fix sealing CommitKey with the PREVIOUS key -
works against code this project did not write.

1. **A new key.** The key fingerprint after the commit differs from the one before.
2. **The same key visualization.** Official clients draw the picture from the chat's
   FIRST shared key and never redraw it (core.telegram.org/api/end-to-end/pfs), so a
   rekey that changed it would show the operator a different picture for the same chat.
3. **The new key works both ways.** A fresh code goes out after the commit, and the
   operator's reply carrying it is read here.
4. **The previous key is discarded - after that reply, not before.** TDLib as acceptor
   sends no confirmation of its own after CommitKey (.ai/RESEARCH/live-rekey.md), so the
   first message under the new key from the far side is the operator's reply. Asserting
   the discard earlier would fail against a correct peer.

The operator's work is the same as the round-trip test: accept the chat, send one reply.
The rekey itself is automatic on the far side.
"""

import os
import secrets

import pytest

from ._live import HUMAN_DEADLINE, await_event, ready_chat

pytestmark = pytest.mark.timeout(600)

ACCEPT_KEY = "DecryptedMessageActionAcceptKey"


async def test_a_rekey_started_here_is_accepted_by_the_official_client(manager, peer, announce):
    chat, _ = await ready_chat(manager, peer, announce, "rekey")
    code = os.environ.get("TSC_TEST_NONCE") or f"tsc-rk-{secrets.token_hex(3)}"
    try:
        before = manager.status(chat.id)
        fingerprint_before, visualization_before = before.key_fingerprint, before.key_hash
        assert visualization_before is not None, "the chat has no key visualization to keep"

        await manager.rekey(chat.id)
        # No operator step: the official client accepts a rekey on its own. The commit
        # is sent while this side handles AcceptKey, before the event is emitted.
        await await_event(
            manager.recorder,
            "ServiceActionReceived",
            where=lambda one: one.chat_id == chat.id and one.action_name == ACCEPT_KEY,
            deadline=HUMAN_DEADLINE,
            missing="the official client never accepted the rekey (no AcceptKey arrived)",
        )

        after = manager.status(chat.id)
        assert (
            after.key_fingerprint != fingerprint_before
        ), "the rekey was accepted but this side kept the same key"
        assert (
            after.key_hash == visualization_before
        ), "the key visualization changed: the far side would draw a different picture"

        await manager.send_message(chat.id, f"rekey {code}")
        announce(
            f"On the second account, REPLY in the same secret chat with the code: {code}. "
            f"Waiting up to {HUMAN_DEADLINE:.0f}s."
        )
        await await_event(
            manager.recorder,
            "MessageReceived",
            where=lambda one: one.chat_id == chat.id and code in (one.text or ""),
            deadline=HUMAN_DEADLINE,
            missing=(
                f"no reply carrying {code} arrived after the rekey - the new key did not "
                "work in one of the two directions"
            ),
        )

        # The reply was the first message under the new key from the far side, and it
        # is what lets this side drop the previous key.
        assert (
            manager.status(chat.id).previous_key is None
        ), "the reply arrived but the previous key was kept"
    finally:
        await manager.close(chat.id, reason="interop rekey check finished")
