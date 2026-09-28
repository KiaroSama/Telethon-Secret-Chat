"""The interop tier needs two real Telegram accounts, so it skips by default.

It skips with the REASON stated rather than silently: a tier that quietly collects
zero tests looks exactly like a tier that passed, and this is the tier carrying the
only evidence that an official client can read what this package writes (README, Correctness).

``pytest_collection_modifyitems`` is a SESSION hook even when it lives in a
subdirectory's conftest - pytest hands it every item it collected, not the ones
under this folder. Filtering by path is therefore not tidiness, it is the whole
correctness of the hook: without it one unset environment variable skipped the
entire repository and a full run reported "42 skipped" with nothing passed, which
is the false green the constitution's Principle III names in so many words.
"""

import os
from pathlib import Path

import pytest

REQUIRED = (
    "TSC_TEST_SESSION",
    "TSC_TEST_PEER",
    # A StringSession carries the authorization, never the application identity,
    # so Telethon still needs both of these on every construction. Listing them
    # here rather than letting the client raise keeps the failure a skip with a
    # reason instead of an error four frames inside a fixture.
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
            "interop needs a real account: "
            + ", ".join(missing)
            + " unset. This tier is never run in CI - a session string in a CI secret "
            "is a credential one misconfigured log away from being public."
        )
    )
    for item in items:
        if HERE == Path(item.path).parent or HERE in Path(item.path).parents:
            item.add_marker(skip)


# --- the live fixtures ---------------------------------------------------------
# Here rather than in `_live.py` so nothing has to import them: a module that both
# imports a fixture by name and takes it as an argument shadows its own import.

import pytest as _pytest  # noqa: E402 - the skip hook above must stay at the top
from telethon import TelegramClient  # noqa: E402
from telethon.sessions import StringSession  # noqa: E402
from telethon.tl import types  # noqa: E402

from telethon_secret_chat import MemoryStorage, SecretChatManager  # noqa: E402

from ._live import Recorder, env, write_capture  # noqa: E402


@_pytest.fixture
def announce(capsys):
    """Print an instruction to the operator, past pytest's capture.

    Without `capsys.disabled()` the line telling them what to do (reply with a
    code, open a file) is buffered until the test ends, which is after the deadline it was
    meant to beat.
    """

    def say(text: str) -> None:
        with capsys.disabled():
            print(f"\n  >> {text}", flush=True)

    return say


@_pytest.fixture
async def client():
    """The test account, connected. Disconnected however the test ends.

    `api_id`/`api_hash` come from the environment beside the session because a
    StringSession does not carry them - Telethon needs the application identity on
    every construction, and a test that hard-coded someone else's would be signing
    in as an application its operator never registered.
    """
    # `from None` on both: the constructor's own error would carry the value as an
    # argument, and pytest's long traceback prints arguments - a malformed session
    # string would land in the operator's scrollback.
    try:
        session = StringSession(env("TSC_TEST_SESSION"))
    except Exception:
        raise RuntimeError("TSC_TEST_SESSION is not a valid StringSession") from None
    try:
        api_id = int(env("TSC_TEST_API_ID"))
    except ValueError:
        raise RuntimeError("TSC_TEST_API_ID is not an integer") from None
    one = TelegramClient(session, api_id, env("TSC_TEST_API_HASH"))
    await one.connect()
    if not await one.is_user_authorized():
        _pytest.fail("TSC_TEST_SESSION is not authorized - the string is stale or was revoked")
    try:
        yield one
    finally:
        await one.disconnect()


@_pytest.fixture
async def manager(client):
    """A started manager with an in-memory store, recording everything it emits.

    `MemoryStorage` rather than `FileStorage` on purpose: an interop run creates
    throwaway chats, and writing their keys into a file that outlives the run leaves
    key material on the operator's disk for no benefit to the assertion.
    """
    one = SecretChatManager(client, MemoryStorage())
    recorder = Recorder()

    for name in (
        "ChatRequested",
        "ChatReady",
        "ChatClosedEvent",
        "MessageReceived",
        "MessageAcknowledged",
        "ServiceActionReceived",
        "DecryptFailed",
    ):
        one.on(name, recorder.events.append)

    async def watch(update):
        chat = getattr(update, "chat", None)
        if isinstance(update, types.UpdateEncryption) and isinstance(chat, types.EncryptedChat):
            recorder.peer_fingerprints[chat.id] = chat.key_fingerprint
        if isinstance(update, types.UpdateNewEncryptedMessage):
            message = update.message
            recorder.frames.setdefault(message.chat_id, []).append(bytes(message.bytes))

    client.add_event_handler(watch)
    await one.start()
    one.recorder = recorder  # the tests read it off the manager they already hold
    try:
        yield one
    finally:
        client.remove_event_handler(watch)
        await one.stop()


@_pytest.fixture
def capture(manager):
    """Opt-in (``TSC_CAPTURE_DIR``): keep the official client's frames as a vector.

    A test calls ``capture.arm(chat_id, reply_contains)`` once the reply arrived;
    the file is written at teardown, after the test's own ``finally`` closed the chat,
    and refused if it is not closed. It holds the chat's key - burned by that close,
    so nothing it protected still exists - the direction, the frames the official
    client wrote, and what this package read from each. No chat id, user, username
    or session: ``tests/vectors/test_official_client_frames.py`` replays it offline.
    """
    target = os.environ.get("TSC_CAPTURE_DIR")
    armed = {}

    class Capture:
        @staticmethod
        def arm(chat_id: int, reply_contains: str) -> None:
            if target:
                chat = manager._chats[chat_id]
                armed.update(chat_id=chat_id, key=chat.key, is_outbound=chat.is_outbound)
                armed["reply_contains"] = reply_contains

    yield Capture
    if target and armed:
        write_capture(manager, armed, Path(target))


@_pytest.fixture
async def peer(client):
    """The second account, resolved once."""
    who = env("TSC_TEST_PEER")
    try:
        return await client.get_entity(int(who))
    except ValueError:
        return await client.get_entity(who)
