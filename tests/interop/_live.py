"""Shared helpers for the interop tier: bounded waits and the chat every test needs.

The fixtures live in ``conftest.py`` beside this file, not here. Importing a fixture
by name into a module that also takes it as an argument shadows it, which reads to a
linter as a redefinition and to a human as two different things with one name; pytest
collects conftest fixtures for free and nothing has to be imported at all.

What is left here is the part that is not a fixture:

**The far end is a person.** ``TSC_TEST_PEER`` names a second account operated by
hand in an official Telegram client, because that is the claim the tier exists to
support - SC-001 says an official client can read what this package writes, and a
second copy of this package reading its own output would not say that. So a step the
human has to perform is announced, and then waited for with a deadline. A wait
without one is how an opt-in tier becomes a hung terminal nobody runs again.
"""

from __future__ import annotations

import asyncio
import os
import time
from dataclasses import dataclass, field
from typing import Any, Callable, List, Optional

import pytest

# How long a step that needs a human is given. Generous on purpose: the operator has
# to unlock a phone and find the chat, and a deadline that fails while they are still
# reaching for it teaches them to raise it rather than to read it.
HUMAN_DEADLINE = 180.0

POLL = 0.25


def env(name: str) -> str:
    value = os.environ.get(name)
    if not value:  # pragma: no cover - conftest skips before this can fire
        raise RuntimeError(
            f"{name} is unset; tests/interop/conftest.py should have skipped"
        )
    return value


@dataclass
class Recorder:
    """Every event the manager emitted, in order, with the raw peer fingerprint.

    ``peer_fingerprints`` is the one thing the manager's own events cannot supply.
    ``ChatReady.key_fingerprint`` is what THIS end computed from the shared secret;
    the number the FAR end computed arrives separately, on the raw
    ``UpdateEncryption`` carrying ``encryptedChat``. Holding both is what lets a test
    assert the two ends agree rather than assert this end agrees with itself.
    """

    events: List[Any] = field(default_factory=list)
    peer_fingerprints: dict = field(default_factory=dict)

    def of(self, kind: str) -> List[Any]:
        return [one for one in self.events if type(one).__name__ == kind]

    def first(
        self, kind: str, where: Optional[Callable[[Any], bool]] = None
    ) -> Optional[Any]:
        for one in self.of(kind):
            if where is None or where(one):
                return one
        return None


async def await_event(
    recorder: Recorder,
    kind: str,
    *,
    where: Optional[Callable[[Any], bool]] = None,
    deadline: float,
    missing: str,
) -> Any:
    """The event, or a failure naming what did not happen.

    ``missing`` is written for whoever is sitting in front of the terminal, so it says
    what was expected of them rather than which assertion failed. A timeout here is
    almost never a bug in the package: it is a tap that was not tapped.
    """
    until = time.monotonic() + deadline
    while time.monotonic() < until:
        found = recorder.first(kind, where)
        if found is not None:
            return found
        await asyncio.sleep(POLL)
    pytest.fail(f"{missing} (waited {deadline:.0f}s for {kind})")


async def ready_chat(manager, peer, announce, label: str):
    """A chat this end requested and the human accepted, with both ends agreeing.

    Shared rather than written out per test, because every test in this tier needs
    one and the acceptance is the slowest step in the file.
    """
    chat = await manager.create(peer)
    announce(
        f"Open Telegram on the second account and ACCEPT the new secret chat "
        f"({label}). Waiting up to {HUMAN_DEADLINE:.0f}s."
    )
    ready = await await_event(
        manager.recorder,
        "ChatReady",
        where=lambda one: one.chat_id == chat.id,
        deadline=HUMAN_DEADLINE,
        missing="the secret chat was never accepted on the second account",
    )
    return chat, ready
