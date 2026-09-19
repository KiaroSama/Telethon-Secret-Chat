"""T043 / US5: encrypted files, uploaded here and opened by an official client.

§6.4 is the claim under test: the bytes are IGE-encrypted client-side before
``upload.saveFilePart``, so the server stores ciphertext only. Nothing local can
demonstrate that - the unit tier proves the encryption is reversible, and the server
proves nothing at all, because ciphertext it cannot read looks to it like any other
upload. The only evidence is an official client opening the file, and that needs a
human to look at a screen.

**Why the media set is what it is.** Every kind goes out as one shape,
``decryptedMessageMediaDocument``, differing only in ``mime_type`` and the bytes. So
the interesting axis is not how many kinds are listed but whether real, structurally
valid files of different types survive the round trip. Two are synthesized here
because they can be made byte-exact and genuinely valid - a 1x1 JPEG and a 1x1 PNG -
alongside plain text and an opaque binary. Video and audio are NOT synthesized:
a handful of bytes labelled ``video/mp4`` is not a video, and an official client
refusing to play it would say something about the client's error handling rather
than about this package. They are picked up from ``TSC_TEST_MEDIA_DIR`` when the
operator points it at real samples, and reported as not covered when they do not.
Principle III: an unknown outcome is reported as unknown.

The receive direction is verified rather than announced - the fingerprint check in
``files.save`` runs before a byte reaches the disk, so a file that saves at all is a
file whose key and address belonged together.
"""

import base64
import os
from pathlib import Path

import pytest

from ._live import HUMAN_DEADLINE, await_event, ready_chat

pytestmark = pytest.mark.timeout(900)

# A real 1x1 JPEG and a real 1x1 PNG. Byte-exact rather than "close enough": a header
# an official client rejects would fail this test for a reason that has nothing to do
# with the encryption it exists to check.
ONE_PIXEL_JPEG = base64.b64decode(
    "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRof"
    "Hh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAAB"
    "AAAAAAAAAAAAAAAAAAAACf/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AKp//2Q=="
)
ONE_PIXEL_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGA"
    "WjR9awAAAABJRU5ErkJggg=="
)

SYNTHESIZED = (
    ("photo-jpeg", "interop-photo.jpg", "image/jpeg", ONE_PIXEL_JPEG),
    ("photo-png", "interop-photo.png", "image/png", ONE_PIXEL_PNG),
    ("text", "interop-note.txt", "text/plain", b"interop document body\n"),
    ("binary", "interop-blob.bin", "application/octet-stream", bytes(range(256)) * 4),
)


def _operator_samples():
    """Real media the operator supplied, or nothing.

    A directory rather than one variable per kind: the set of formats worth checking
    changes with what the operator's client version actually renders, and enumerating
    them here would freeze that list into the test.
    """
    where = os.environ.get("TSC_TEST_MEDIA_DIR")
    if not where:
        return []
    root = Path(where)
    if not root.is_dir():
        pytest.fail(f"TSC_TEST_MEDIA_DIR points at {where}, which is not a directory")
    return sorted(one for one in root.iterdir() if one.is_file())


async def test_every_media_kind_is_sent_and_opens_on_the_far_side(
    manager, peer, announce, tmp_path
):
    chat, _ = await ready_chat(manager, peer, announce, "media")
    sent = []
    try:
        for label, name, mime, body in SYNTHESIZED:
            source = tmp_path / name
            source.write_bytes(body)
            await manager.send_file(chat.id, source, caption=label, mime_type=mime)
            sent.append(name)

        extras = _operator_samples()
        for source in extras:
            await manager.send_file(chat.id, source, caption=f"sample {source.name}")
            sent.append(source.name)

        if not extras:
            announce(
                "No TSC_TEST_MEDIA_DIR was set, so video and audio were NOT covered "
                "by this run. Point it at a directory of real samples to include them."
            )

        announce(
            f"On the second account, open each of the {len(sent)} files that just "
            f"arrived: {', '.join(sent)}. Then reply with the single word `ok` if "
            f"every one opened, or with the name of the first that did not. "
            f"Waiting up to {HUMAN_DEADLINE:.0f}s."
        )
        verdict = await await_event(
            manager.recorder,
            "MessageReceived",
            where=lambda one: one.chat_id == chat.id and (one.text or "").strip(),
            deadline=HUMAN_DEADLINE,
            missing="the operator never reported whether the files opened",
        )

        answer = (verdict.text or "").strip().lower()
        assert answer == "ok", (
            f"the operator reported a file that did not open on the far side: "
            f"{verdict.text!r}"
        )
    finally:
        await manager.close(chat.id, reason="interop media check finished")


async def test_a_file_from_the_far_side_decrypts_and_is_written(
    manager, peer, announce, tmp_path
):
    chat, _ = await ready_chat(manager, peer, announce, "incoming file")
    try:
        announce(
            f"On the second account, SEND any file into the secret chat. "
            f"Waiting up to {HUMAN_DEADLINE:.0f}s."
        )
        arrived = await await_event(
            manager.recorder,
            "MessageReceived",
            where=lambda one: (
                one.chat_id == chat.id
                and one.media is not None
                and one.file is not None
            ),
            deadline=HUMAN_DEADLINE,
            missing="no file arrived from the second account",
        )

        target = tmp_path / "received.bin"
        written = await manager.save_file(arrived, target)

        # `save` verifies the fingerprint BEFORE writing, so reaching a written file
        # at all is the assertion: the key inside the message and the address outside
        # it belonged together.
        assert written.exists(), "save_file returned a path that does not exist"
        assert written.stat().st_size == arrived.media.size, (
            f"the decrypted file is {written.stat().st_size} bytes, but the message "
            f"declared {arrived.media.size}"
        )
    finally:
        await manager.close(chat.id, reason="interop incoming file finished")
