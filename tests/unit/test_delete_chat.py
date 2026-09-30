"""Deleting a secret chat, on this side only or on both sides (spec 007).

Oracle: TDLib ``SecretChatActor::cancel_chat(delete_history)`` - close, flush the local
history, then ``messages.discardEncryption(delete_history)``; an incoming
``encryptedChatDiscarded.history_deleted`` flushes the same way without a request.
"""

import pytest
from telethon.tl import functions

from telethon_secret_chat import ChatState, ManagerStopping, UnknownChat

from .fake_client import establish


def discards(client):
    return [r for r in client.sent if isinstance(r, functions.messages.DiscardEncryptionRequest)]


def holds_nothing(manager, storage, chat_id):
    assert chat_id not in [snapshot.id for snapshot in manager.list()]
    assert storage.load(chat_id) is None
    assert storage.retained_out(chat_id) == []
    assert storage.peek_in(chat_id) == []
    assert chat_id not in manager._history


@pytest.fixture
async def open_chat(pair):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    await b.send_message(chat_a.id, "so the history is not empty")
    closed_b = []
    b.on("ChatClosedEvent", closed_b.append)
    return wire, a, b, chat_a.id, closed_b


async def test_both_sides_on_an_open_chat(open_chat):
    wire, a, b, chat_id, closed_b = open_chat
    assert await a.delete_secret_chat_both_sides(chat_id) is True
    (request,) = discards(wire.a)
    assert request.delete_history is True
    holds_nothing(a, a._storage, chat_id)
    # The peer's side goes too, and its application is told the history went with it.
    holds_nothing(b, b._storage, chat_id)
    assert [event.history_deleted for event in closed_b] == [True]


async def test_both_sides_on_a_tombstone_still_asks_and_reports_the_refusal(open_chat):
    wire, a, b, chat_id, _ = open_chat
    await a.close(chat_id)
    assert a.status(chat_id).state is ChatState.CLOSED
    assert await a.delete_secret_chat_both_sides(chat_id) is False
    assert [r.delete_history for r in discards(wire.a)] == [False, True]
    holds_nothing(a, a._storage, chat_id)


async def test_one_side_on_an_open_chat(open_chat):
    wire, a, b, chat_id, closed_b = open_chat
    await a.delete_secret_chat(chat_id)
    (request,) = discards(wire.a)
    assert request.delete_history is False
    holds_nothing(a, a._storage, chat_id)
    # The peer only sees the chat end; its tombstone and history stay.
    assert b.status(chat_id).state is ChatState.CLOSED
    assert [event.history_deleted for event in closed_b] == [False]


async def test_one_side_on_a_tombstone_sends_nothing(open_chat):
    wire, a, _, chat_id, _ = open_chat
    await a.close(chat_id)
    await a.delete_secret_chat(chat_id)
    assert len(discards(wire.a)) == 1  # the close's own, nothing more
    holds_nothing(a, a._storage, chat_id)


@pytest.mark.parametrize("method", ["delete_secret_chat", "delete_secret_chat_both_sides"])
async def test_unknown_and_repeated_deletes_raise(open_chat, method):
    wire, a, _, chat_id, _ = open_chat
    with pytest.raises(UnknownChat):
        await getattr(a, method)(999)
    await getattr(a, method)(chat_id)
    sent = len(wire.a.sent)
    with pytest.raises(UnknownChat):
        await getattr(a, method)(chat_id)
    assert len(wire.a.sent) == sent


async def test_a_storage_failure_keeps_the_chat_installed(open_chat, monkeypatch):
    _, a, _, chat_id, _ = open_chat
    await a.close(chat_id)

    def refuse(chat_id):
        raise OSError("disk full")

    monkeypatch.setattr(a._storage, "delete", refuse)
    with pytest.raises(OSError):
        await a.delete_secret_chat(chat_id)
    assert a.status(chat_id).state is ChatState.CLOSED


async def test_deleting_while_stopping_is_refused(open_chat):
    _, a, _, chat_id, _ = open_chat
    a._stopping = True
    try:
        with pytest.raises(ManagerStopping):
            await a.delete_secret_chat(chat_id)
    finally:
        a._stopping = False
    assert a.status(chat_id).state is ChatState.READY


async def test_a_peer_delete_removes_a_tombstone_here_too(open_chat):
    wire, a, b, chat_id, closed_b = open_chat
    await b.close(chat_id)  # b keeps a tombstone
    wire.a.discarded.clear()
    await a.delete_secret_chat_both_sides(chat_id)
    holds_nothing(b, b._storage, chat_id)
    assert [event.history_deleted for event in closed_b] == [False, True]


async def test_a_plain_peer_close_still_leaves_a_tombstone(open_chat):
    _, a, b, chat_id, closed_b = open_chat
    await a.close(chat_id)
    assert b.status(chat_id).state is ChatState.CLOSED
    assert [event.history_deleted for event in closed_b] == [False]
