"""A reply points at a random id, because the encrypted layer has no message ids.

The schema carried ``reply_to_random_id`` from the start (§7.1) and neither send path
passed it, so every reply this package sent arrived as an ordinary message. That is
the same silent loss the caption and voice-note work was done to remove, in a
different field: the sender sees a reply, the peer sees a remark.

Both ends are this package, so what these prove is that the field survives the round
trip - schema round-trip coverage pins the wire bytes.
"""

import pytest

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import Wire, establish


@pytest.fixture
async def pair():
    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage())
    await a.start()
    await b.start()
    yield wire, a, b
    await a.stop()
    await b.stop()


async def test_a_text_reply_reaches_the_peer_as_a_reply(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)

    answered = await a.send_message(chat_a.id, "the question")
    await a.send_message(chat_a.id, "the answer", reply_to=answered)

    received = b.read_history(chat_b.id)
    assert received[-1].reply_to == answered, "the reply arrived as an ordinary message"


async def test_an_ordinary_message_replies_to_nothing(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)

    await a.send_message(chat_a.id, "just talking")

    assert b.read_history(chat_b.id)[-1].reply_to is None


async def test_a_file_reply_reaches_the_peer_as_a_reply(pair, tmp_path):
    """Files take a different send path, so the field is wired twice and proved twice."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    target = tmp_path / "note.txt"
    target.write_bytes(b"bytes")

    answered = await a.send_message(chat_a.id, "the question")
    await a.send_file(chat_a.id, target, reply_to=answered)

    assert b.read_history(chat_b.id)[-1].reply_to == answered
