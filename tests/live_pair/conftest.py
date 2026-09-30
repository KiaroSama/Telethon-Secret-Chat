"""The pair tier needs two real Telegram accounts, so it skips by default, saying why.

Run it only through ``scripts/run_interop.ps1 -PeerAccount``: the launcher reads both
session strings from the private ``.env`` into this process alone and restores the
environment afterwards. Nothing here prints, logs or stores a session value.

The skip hook filters by path for the reason given in ``tests/interop/conftest.py``: a
collection hook in a subdirectory still receives every item of the session.
"""

import os
import shutil
import tempfile
from pathlib import Path

import pytest

REQUIRED = (
    "TSC_PAIR_SESSION_A",
    "TSC_PAIR_SESSION_B",
    "TSC_PAIR_PEER",
    "TSC_TEST_API_ID",
    "TSC_TEST_API_HASH",
)

HERE = Path(__file__).parent


def pytest_collection_modifyitems(config, items):
    missing = [name for name in REQUIRED if not os.environ.get(name)]
    if not missing:
        return
    skip = pytest.mark.skip(
        reason=(
            "the pair tier needs two real accounts: "
            + ", ".join(missing)
            + " unset. Run it with scripts/run_interop.ps1 -PeerAccount; it never runs in CI."
        )
    )
    for item in items:
        if HERE == Path(item.path).parent or HERE in Path(item.path).parents:
            item.add_marker(skip)


import pytest as _pytest  # noqa: E402 - the skip hook above must stay at the top
from telethon import TelegramClient  # noqa: E402
from telethon.errors import FloodWaitError  # noqa: E402
from telethon.sessions import StringSession  # noqa: E402

from telethon_secret_chat import FileStorage, MemoryStorage  # noqa: E402

from ._pair import DEADLINE, started_manager  # noqa: E402
from interop._live import await_event, env  # noqa: E402


async def _connect(name: str):
    # `from None`: a constructor error would carry the value, and a traceback prints it.
    try:
        session = StringSession(env(name))
    except Exception:
        raise RuntimeError(f"{name} is not a valid StringSession") from None
    try:
        api_id = int(env("TSC_TEST_API_ID"))
    except ValueError:
        raise RuntimeError("TSC_TEST_API_ID is not an integer") from None
    client = TelegramClient(session, api_id, env("TSC_TEST_API_HASH"))
    await client.connect()
    if not await client.is_user_authorized():
        await client.disconnect()
        _pytest.fail(f"{name} is not authorized - the string is stale or was revoked")
    return client


@_pytest.fixture
async def client_a():
    client = await _connect("TSC_PAIR_SESSION_A")
    try:
        yield client
    finally:
        await client.disconnect()


@_pytest.fixture
async def client_b():
    client = await _connect("TSC_PAIR_SESSION_B")
    try:
        yield client
    finally:
        await client.disconnect()


@_pytest.fixture
def store_b():
    """The accepting side's store: a file, so the restart case can reopen it.

    A private temp directory removed at teardown: the keys in it protect only this
    run's chats, which are closed by then, and nothing of them outlives the run.
    """
    directory = tempfile.mkdtemp(prefix="tsc-pair-")
    try:
        yield Path(directory) / "store-b.json"
    finally:
        shutil.rmtree(directory, ignore_errors=True)


@_pytest.fixture
async def manager_a(client_a):
    manager = await started_manager(client_a, MemoryStorage(), accept=False)
    try:
        yield manager
    finally:
        await manager.stop()


@_pytest.fixture
async def manager_b(client_b, store_b):
    manager = await started_manager(client_b, FileStorage(store_b), accept=True)
    try:
        yield manager
    finally:
        await manager.stop()


@_pytest.fixture
async def pair_chat(client_a, manager_a, manager_b):
    """A chat the starting account requested and the accepting account accepted.

    Yields ``(chat_id, ready_a, ready_b)``. The chat is closed from the starting side
    whatever the test did, so no run leaves a chat open on either account.
    """
    try:
        peer = await client_a.get_input_entity(env("TSC_PAIR_PEER"))
    except Exception as failure:
        _pytest.fail(
            "the starting account cannot find the accepting account by TSC_PAIR_PEER "
            f"({type(failure).__name__}); pass the accepting account's public username"
        )
    try:
        chat = await manager_a.create(peer)
    except FloodWaitError as limit:
        _pytest.fail(
            f"Telegram rate-limits new secret chats: wait {limit.seconds}s, then run again"
        )
    try:
        ready_a = await await_event(
            manager_a.recorder,
            "ChatReady",
            where=lambda one: one.chat_id == chat.id,
            deadline=DEADLINE,
            missing="the accepting account never completed the key exchange "
            f"(accept failures: {manager_b.accept_failures or 'none'})",
        )
        ready_b = await await_event(
            manager_b.recorder,
            "ChatReady",
            where=lambda one: one.chat_id == chat.id,
            deadline=DEADLINE,
            missing="the accepting side never reported the chat ready",
        )
        yield chat.id, ready_a, ready_b
    finally:
        # A delete case (test_pair_delete.py) leaves nothing to close.
        if chat.id in [one.id for one in manager_a.list()]:
            await manager_a.close(chat.id, reason="pair tier case finished")
