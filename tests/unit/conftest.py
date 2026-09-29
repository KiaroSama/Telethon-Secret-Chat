"""Fixtures shared by the unit tier."""

import os

import pytest

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import Wire


@pytest.fixture
async def pair():
    """Two started managers joined by one fake server; torn down whatever happens."""
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    try:
        yield wire, a, b
    finally:
        try:
            await a.stop()
        finally:
            await b.stop()
        wire.a.assert_quiet()
        wire.b.assert_quiet()


@pytest.fixture
async def chat(pair):
    """Two established managers, plus a sender that returns what arrived."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)

    async def send(tmp_path, name, **kwargs):
        source = tmp_path / name
        source.write_bytes(os.urandom(64))
        await a.send_file(chat_a.id, source, **kwargs)
        return got[-1]

    return wire, a, chat_a, send
