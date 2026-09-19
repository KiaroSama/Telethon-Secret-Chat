"""SC-006 and FR-014 - a chat survives the process, and the conversation continues.

"Given an established chat, When the application restarts, Then the chat is still
usable and message ordering continues where it left off" (US1, scenario 5).

The restart is real here: the manager is destroyed and a new one is built over the
SAME backend, the way a process restart would. What must come back is the key, the
fingerprint, BOTH counters and the layer - and the conversation must continue at the
counter it stopped at, because §3.5 says the peer drops anything at or below the
sequence number it has already seen. A chat that restarts at zero is a chat whose
next twenty messages are silently discarded by the far end.
"""

import pytest

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.storage import FileStorage, MemoryStorage

from .fake_client import Wire, establish


async def a_pair(store_a, store_b):
    wire = Wire()
    a = SecretChatManager(wire.a, storage=store_a)
    b = SecretChatManager(wire.b, storage=store_b)
    await a.start()
    await b.start()
    return wire, a, b


async def test_a_chat_comes_back_with_its_key_and_counters(tmp_path):
    store_a, store_b = FileStorage(tmp_path / "a.db"), MemoryStorage()
    wire, a, b = await a_pair(store_a, store_b)
    chat_a, _ = await establish(a, b, wire)

    await a.send_message(chat_a.id, "one")
    await a.send_message(chat_a.id, "two")
    before = a.status(chat_a.id)
    await a.stop()

    # The process ends here. Everything below is a new manager over the same file.
    revived = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    await revived.start()
    after = revived.status(chat_a.id)

    assert after.key == before.key
    assert after.key_fingerprint == before.key_fingerprint
    assert after.in_seq_no == before.in_seq_no
    assert after.out_seq_no == before.out_seq_no
    assert after.layer == before.layer
    assert after.is_outbound == before.is_outbound
    assert after.state is ChatState.READY
    await revived.stop()
    await b.stop()


async def test_the_conversation_continues_in_order_across_the_restart(tmp_path):
    """The counter, not just the key. This is the test that fails when a manager
    reloads a chat and starts counting from zero."""
    store_b = MemoryStorage()
    wire, a, b = await a_pair(FileStorage(tmp_path / "a.db"), store_b)
    chat_a, chat_b = await establish(a, b, wire)

    received = []
    b.on("MessageReceived", lambda e: received.append(e.text))

    await a.send_message(chat_a.id, "before the restart")
    await a.stop()

    revived = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    await revived.start()
    await revived.send_message(chat_a.id, "after the restart")

    assert received == ["before the restart", "after the restart"]
    await revived.stop()
    await b.stop()


async def test_a_second_manager_sees_every_chat_the_backend_holds(tmp_path):
    wire, a, b = await a_pair(FileStorage(tmp_path / "a.db"), MemoryStorage())
    chat_a, _ = await establish(a, b, wire)
    await a.stop()

    revived = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    await revived.start()
    assert [c.id for c in revived.list()] == [chat_a.id]
    await revived.stop()
    await b.stop()


async def test_a_closed_chat_stays_closed_across_a_restart(tmp_path):
    """data-model.md §1: terminal means terminal, and a restart is not a loophole."""
    wire, a, b = await a_pair(FileStorage(tmp_path / "a.db"), MemoryStorage())
    chat_a, _ = await establish(a, b, wire)
    await a.close(chat_a.id)
    await a.stop()

    revived = SecretChatManager(wire.a, storage=FileStorage(tmp_path / "a.db"))
    await revived.start()
    assert revived.status(chat_a.id).state is ChatState.CLOSED
    await revived.stop()
    await b.stop()


async def test_the_backend_is_required(tmp_path):
    """FR-014, and the constitution's "there MUST NOT be a silent fallback that
    writes keys into the current working directory"."""
    from telethon_secret_chat.errors import StorageRequired

    with pytest.raises(StorageRequired):
        SecretChatManager(Wire().a, storage=None)


async def test_start_and_stop_are_idempotent(tmp_path):
    """contracts/public-api.md §1: "Both are idempotent." A second ``start`` that
    subscribed twice would deliver every message twice."""
    wire, a, b = await a_pair(MemoryStorage(), MemoryStorage())
    await a.start()
    await a.start()
    assert len(wire.a.handlers) == 1
    await a.stop()
    await a.stop()
    assert wire.a.handlers == []
    await b.stop()


async def test_the_manager_does_not_mutate_the_client():
    """contracts/public-api.md §1: "The manager never installs a global handler and
    never mutates client." §8.0 measured the archived package reaching into four
    private attributes and mutating a process-wide registry."""
    wire = Wire()
    before = set(vars(wire.a))
    manager = SecretChatManager(wire.a, storage=MemoryStorage())
    await manager.start()
    await manager.stop()
    assert set(vars(wire.a)) - before <= {"handlers"}
