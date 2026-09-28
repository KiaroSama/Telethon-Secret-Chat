"""Shared helpers for the interop tier: bounded waits and the chat every test needs.

The fixtures live in ``conftest.py`` beside this file, not here. Importing a fixture
by name into a module that also takes it as an argument shadows it, which reads to a
linter as a redefinition and to a human as two different things with one name; pytest
collects conftest fixtures for free and nothing has to be imported at all.

What is left here is the part that is not a fixture:

**The far end is a person.** ``TSC_TEST_PEER`` names a second account operated by
hand in an official Telegram client, because that is the claim the tier exists to
support - that an official client can read what this package writes (README,
Correctness) - and a second copy of this package reading its own output would not say
that. So a step the
human has to perform is announced, and then waited for with a deadline. A wait
without one is how an opt-in tier becomes a hung terminal nobody runs again.
"""

from __future__ import annotations

import asyncio
import datetime
import json
import os
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, List, Optional

import pytest

from telethon_secret_chat import crypto, framing

# How long a step that needs a human is given. Generous on purpose: the operator has
# to unlock a phone and find the chat, and a deadline that fails while they are still
# reaching for it teaches them to raise it rather than to read it.
HUMAN_DEADLINE = 180.0

POLL = 0.25


def env(name: str) -> str:
    value = os.environ.get(name)
    if not value:  # pragma: no cover - conftest skips before this can fire
        raise RuntimeError(f"{name} is unset; tests/interop/conftest.py should have skipped")
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
    #: Raw ``EncryptedMessage.bytes`` per chat, in arrival order: frames the official
    #: client wrote. Held in memory only; written out solely by the capture mode.
    frames: dict = field(default_factory=dict)

    def of(self, kind: str) -> List[Any]:
        return [one for one in self.events if type(one).__name__ == kind]

    def first(self, kind: str, where: Optional[Callable[[Any], bool]] = None) -> Optional[Any]:
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

    # What DID arrive is the diagnosis. Waiting for `MessageReceived` and reporting
    # only its absence hides the case that matters most: the message arrived and
    # failed to decrypt, which emits `DecryptFailed` and is a completely different
    # bug from nothing arriving at all. The live run that found the missing `await`
    # in `_parse_text` then stalled here with no clue which of the two it was.
    failures = recorder.of("DecryptFailed")
    seen = ", ".join(sorted({type(e).__name__ for e in recorder.events})) or "nothing"
    detail = ""
    if failures:
        detail = " DECRYPT FAILED instead: " + "; ".join(
            str(getattr(f, "reason", "?")) for f in failures[:3]
        )
    pytest.fail(
        f"{missing} (waited {deadline:.0f}s for {kind}). "
        f"Events that did arrive: {seen}.{detail}"
    )


async def ready_chat(manager, peer, announce, label: str):
    """A chat this end requested and the peer's device accepted, with both ends agreeing.

    Shared rather than written out per test, because every test in this tier needs
    one and the acceptance is the slowest step in the file.

    Accepting is not a manual step: the request reaches every device the peer has and
    the first to complete the key exchange wins. The operator only keeps one online.

    A chat that never becomes ready is closed here, before the failure propagates. The
    tests' own ``finally`` blocks start only after this returns, so without it every
    timed-out request stayed pending on the test accounts.
    """
    chat = await manager.create(peer)
    announce(
        f"Keep a device of the second account online; it accepts the new secret chat "
        f"({label}) on its own. Waiting up to {HUMAN_DEADLINE:.0f}s."
    )
    try:
        ready = await await_event(
            manager.recorder,
            "ChatReady",
            where=lambda one: one.chat_id == chat.id,
            deadline=HUMAN_DEADLINE,
            missing="the secret chat was never accepted - was a device of the second "
            "account online?",
        )
    except BaseException:
        # BaseException: pytest.fail raises Failed (not an Exception subclass) and a
        # Ctrl+C mid-wait must not leave the request behind either.
        await manager.close(chat.id, reason="interop acceptance timed out")
        raise
    return chat, ready


def write_capture(manager, armed: dict, directory: Path) -> Path:
    """Write the capture fixture; see the ``capture`` fixture in conftest.py.

    Refuses unless the chat is closed: the provenance rule is that the key in the
    file protected a throwaway chat that no longer exists. The expected fields are
    what this package read from each frame at capture time, so the offline test pins
    that reading against bytes an official client wrote; ``reply_contains`` is the
    independent check, the code the operator typed.
    """
    chat = manager._chats[armed["chat_id"]]
    if chat.state.value != "closed":
        pytest.fail("capture refused: the chat is not closed, so its key is not burned")
    x = 8 if armed["is_outbound"] else 0
    frames = []
    for frame in manager.recorder.frames.get(armed["chat_id"], []):
        wrapper = framing.unwrap(crypto.decrypt_frame(armed["key"], frame, x))
        inner = wrapper.message
        frames.append(
            {
                "frame": frame.hex(),
                "expected": {
                    "layer": wrapper.layer,
                    "in_seq_no": wrapper.in_seq_no,
                    "out_seq_no": wrapper.out_seq_no,
                    "type": type(inner).__name__,
                    "text": getattr(inner, "message", None),
                },
            }
        )
    document = {
        "provenance": "throwaway interop chat, closed before this file was written",
        "key": armed["key"].hex(),
        "is_outbound": armed["is_outbound"],
        # This side's counter at the end, so the offline replay accepts the peer's echo.
        "local_out_seq_no": chat.out_seq_no,
        "reply_contains": armed["reply_contains"],
        "frames": frames,
    }
    directory.mkdir(parents=True, exist_ok=True)
    stem = f"official-client-{datetime.date.today().isoformat()}"
    path = directory / f"{stem}.json"
    for n in range(2, 100):
        if not path.exists():
            break
        path = directory / f"{stem}-{n}.json"
    path.write_text(json.dumps(document, indent=1) + "\n", encoding="utf-8", newline="\n")
    return path
