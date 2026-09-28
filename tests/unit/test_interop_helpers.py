"""The interop helpers clean up after themselves without a live account."""

from types import SimpleNamespace

import pytest

from tests.interop import _live


class _Manager:
    """Just enough of a manager for ``ready_chat``: a create and a recorded close."""

    def __init__(self):
        self.recorder = _live.Recorder()
        self.closed = []

    async def create(self, peer):
        return SimpleNamespace(id=7)

    async def close(self, chat_id, reason=None):
        self.closed.append((chat_id, reason))


async def test_a_chat_that_is_never_accepted_is_closed(monkeypatch):
    async def never(*args, **kwargs):
        pytest.fail("the peer never accepted")

    monkeypatch.setattr(_live, "await_event", never)
    manager = _Manager()
    with pytest.raises(pytest.fail.Exception):
        await _live.ready_chat(manager, peer=object(), announce=lambda text: None, label="t")
    assert manager.closed == [(7, "interop acceptance timed out")]


async def test_an_accepted_chat_is_left_open(monkeypatch):
    async def ready(*args, **kwargs):
        return "ready"

    monkeypatch.setattr(_live, "await_event", ready)
    manager = _Manager()
    chat, event = await _live.ready_chat(
        manager, peer=object(), announce=lambda text: None, label="t"
    )
    assert (chat.id, event, manager.closed) == (7, "ready", [])
