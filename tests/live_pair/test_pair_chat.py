"""Pair tier: one chat between two of the owner's accounts, through every stage.

One chat for all four stages on purpose: Telegram rate-limits new secret chats
(``FloodWaitError`` on ``messages.requestEncryption`` after about ten in an hour, measured
2026-09-29), so a chat per stage made the tier unrunnable twice in a row. Each stage
names itself when it fails. Real-server evidence only: both ends are this package.
"""

import asyncio
import os
import time

import pytest
from interop._live import await_event

from telethon_secret_chat import FileStorage

from ._pair import DEADLINE, POLL, await_texts, started_manager

pytestmark = pytest.mark.timeout(420)

SIZES = {"small": 5 * 1024, "multipart": 3 * 512 * 1024 + 11}


async def _text_both_ways(a, b, chat_id, tag):
    from_a = [f"pair {tag} a->b {n}" for n in range(3)]
    from_b = [f"pair {tag} b->a {n}" for n in range(3)]
    for text in from_a:
        await a.send_message(chat_id, text)
    for text in from_b:
        await b.send_message(chat_id, text)
    assert await await_texts(b, chat_id, from_a) == from_a, f"{tag}: a->b out of order"
    assert await await_texts(a, chat_id, from_b) == from_b, f"{tag}: b->a out of order"


async def _rekey(a, b, chat_id):
    first = a.status(chat_id).key_fingerprint
    await a.rekey(chat_id)
    until = time.monotonic() + DEADLINE
    while time.monotonic() < until:
        sa, sb = a.status(chat_id), b.status(chat_id)
        settled = not sa.exchange_in_progress and not sb.exchange_in_progress
        if settled and sa.key_fingerprint == sb.key_fingerprint != first:
            return
        await asyncio.sleep(POLL)
    pytest.fail("rekey: never settled on one new key at both ends")


async def _file(sender, receiver, chat_id, directory, label, size):
    source = directory / f"{label}.bin"
    content = os.urandom(size)
    source.write_bytes(content)
    random_id = await sender.send_file(chat_id, source)
    event = await await_event(
        receiver.recorder,
        "MessageReceived",
        where=lambda one: one.chat_id == chat_id and one.random_id == random_id,
        deadline=DEADLINE,
        missing=f"files: the {label} file never arrived",
    )
    saved = await receiver.save_file(event, directory / "saved" / f"{label}.bin")
    assert saved.read_bytes() == content, f"files: the {label} file changed on the way"


async def test_one_chat_through_text_rekey_files_and_a_restart(
    pair_chat, manager_a, manager_b, client_b, store_b, tmp_path
):
    chat_id, ready_a, ready_b = pair_chat
    assert (
        ready_a.key_fingerprint == ready_b.key_fingerprint
    ), "the two ends derived different keys"
    assert ready_a.key_hash == ready_b.key_hash

    await _text_both_ways(manager_a, manager_b, chat_id, "text")

    await _rekey(manager_a, manager_b, chat_id)
    await _text_both_ways(manager_a, manager_b, chat_id, "after-rekey")

    for label, size in SIZES.items():
        await _file(manager_a, manager_b, chat_id, tmp_path, f"a-{label}", size)
        await _file(manager_b, manager_a, chat_id, tmp_path, f"b-{label}", size)

    await manager_b.stop()
    restarted = await started_manager(client_b, FileStorage(store_b), accept=True)
    try:
        await _text_both_ways(manager_a, restarted, chat_id, "after-restart")
        assert not restarted.recorder.of("DecryptFailed")
    finally:
        await restarted.stop()
    assert not manager_a.recorder.of("DecryptFailed")
    assert not manager_b.recorder.of("DecryptFailed")
