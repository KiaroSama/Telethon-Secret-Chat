"""Fixtures shared by the unit tier."""

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
