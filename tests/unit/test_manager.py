"""contracts/public-api.md - the surface an application actually uses.

Two managers talking to each other through a fake server. That exercises both sides
of §1.6's asymmetry table in one run, which is the cheap way to catch the failure
§1.6 calls "the most likely silent failure": getting the side wrong, so one end
encrypts at ``x = 0`` while the other decrypts expecting the same.

What this tier does NOT prove is that the bytes are right - both ends are this
package (Principle I's "two instances of the same wrong code"). tests/vectors pins
the primitives against Telethon's, and tests/interop against a real client.
"""

import pytest

from telethon.tl import functions

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.errors import ChatClosed, ChatNotReady, ParameterRejected
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


# --- establishment ------------------------------------------------------------


async def test_creating_a_chat_leaves_it_awaiting_acceptance(pair):
    """US1 scenario 1: "the application reports it as awaiting acceptance"."""
    wire, a, _ = pair
    chat = await a.create(2000)
    assert chat.state is ChatState.REQUESTED and chat.key is None


async def test_the_dh_configuration_is_validated_before_a_chat_is_requested(pair):
    """FR-001. A chat established on a bad prime cannot be repaired afterwards."""
    wire, a, _ = pair
    wire.a.dh_prime_override = 2**2047 + 1  # right length, divisible by three
    with pytest.raises(ParameterRejected):
        await a.create(2000)
    assert a.list() == [], "a chat was recorded for parameters that were refused"
    assert not [
        r for r in wire.a.sent if isinstance(r, functions.messages.RequestEncryptionRequest)
    ], "a chat was REQUESTED on parameters that were refused"


async def test_both_ends_report_the_same_fingerprint(pair):
    """US1 scenario 2, and the one check no amount of unit testing replaces at the
    interop tier: "both ends report the SAME key fingerprint"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    assert chat_a.key_fingerprint == chat_b.key_fingerprint
    assert chat_a.key == chat_b.key
    assert chat_a.state is ChatState.READY and chat_b.state is ChatState.READY


async def test_the_two_sides_disagree_about_who_originated(pair):
    """§1.6: exactly one side is the originator, and everything asymmetric follows
    from it."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    assert chat_a.is_outbound is True and chat_b.is_outbound is False
    assert (chat_a.out_x, chat_b.out_x) == (0, 8)


async def test_an_incoming_request_is_reported_to_the_application(pair):
    """contracts §3: ``ChatRequested`` - "an incoming request awaiting accept"."""
    wire, a, b = pair
    seen = []
    b.on("ChatRequested", seen.append)
    await establish(a, b, wire)
    assert [e.chat_id for e in seen] and seen[0].peer_user_id == 1000


async def test_both_ends_are_told_the_chat_is_ready(pair):
    wire, a, b = pair
    ready = []
    a.on("ChatReady", ready.append)
    b.on("ChatReady", ready.append)
    chat_a, _ = await establish(a, b, wire)
    assert len(ready) == 2
    assert {e.key_fingerprint for e in ready} == {chat_a.key_fingerprint}


async def test_a_peer_publishing_the_wrong_fingerprint_is_refused(pair):
    """§1.4: "Otherwise, messages.discardEncryption must be executed and the user
    notified." Not a warning, and not a chat that limps on."""
    from telethon.tl import types

    wire, a, b = pair
    chat = await a.create(2000)
    request = wire.a.sent[-1]
    await wire.a.deliver_update(
        types.UpdateEncryption(
            chat=types.EncryptedChat(
                id=chat.id,
                access_hash=7777,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a_or_b=request.g_a,  # a well-formed value
                key_fingerprint=12345,  # that does not match the key it implies
            ),
            date=0,
        )
    )
    assert a.status(chat.id).state is ChatState.CLOSED


# --- the conversation ---------------------------------------------------------


async def test_a_message_crosses_and_arrives_as_text(pair):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    await a.send_message(chat_a.id, "hello there")
    assert [e.text for e in got] == ["hello there"]


async def test_messages_arrive_in_order(pair):
    """FR-009."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", lambda e: got.append(e.text))
    for i in range(5):
        await a.send_message(chat_a.id, f"message {i}")
    assert got == [f"message {i}" for i in range(5)]


async def test_both_directions_work(pair):
    """The asymmetry check. If ``x`` or the parity were taken from the wrong side,
    one of these two directions would fail and the other would pass."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    to_b, to_a = [], []
    b.on("MessageReceived", lambda e: to_b.append(e.text))
    a.on("MessageReceived", lambda e: to_a.append(e.text))
    await a.send_message(chat_a.id, "from the originator")
    await b.send_message(chat_b.id, "from the acceptor")
    assert to_b == ["from the originator"] and to_a == ["from the acceptor"]


async def test_read_history_returns_what_arrived_in_order(pair):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    for i in range(4):
        await a.send_message(chat_a.id, f"m{i}")
    assert [m.text for m in b.read_history(chat_b.id, limit=10)] == ["m0", "m1", "m2", "m3"]
    assert [m.text for m in b.read_history(chat_b.id, limit=2)] == ["m2", "m3"]


async def test_send_returns_the_message_id(pair):
    """contracts §2: ``send_message`` "resolves when Telegram accepts the
    ciphertext", returning "the sent message's id"."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    assert isinstance(await a.send_message(chat_a.id, "x"), int)


# --- refusals at the surface --------------------------------------------------


async def test_sending_before_the_chat_is_ready_refuses(pair):
    wire, a, _ = pair
    chat = await a.create(2000)
    with pytest.raises(ChatNotReady):
        await a.send_message(chat.id, "too early")


async def test_sending_on_a_closed_chat_refuses_with_a_reason(pair):
    """The spec's edge case: "sending refuses rather than throwing an unrelated
    transport error"."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    await a.close(chat_a.id)
    with pytest.raises(ChatClosed):
        await a.send_message(chat_a.id, "after the end")


async def test_closing_twice_states_it_rather_than_raising(pair):
    """contracts §2: ``close`` refuses when "already closed (states it, does not
    raise)"."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    await a.close(chat_a.id)
    await a.close(chat_a.id)
    assert a.status(chat_a.id).state is ChatState.CLOSED


async def test_an_unknown_chat_is_refused_by_status_and_history(pair):
    wire, a, _ = pair
    with pytest.raises(KeyError):
        a.status(999999)
    with pytest.raises(KeyError):
        a.read_history(999999, limit=1)


async def test_a_message_for_a_chat_we_have_no_key_for_is_refused(pair):
    """The spec's edge case: "A message arrives for a chat this installation has no
    key for: refused, with nothing written to disk"."""
    wire, a, b = pair
    failures = []
    b.on("DecryptFailed", failures.append)
    await wire.b.deliver(424242, b"\x00" * 64)
    assert failures and failures[0].chat_id == 424242
    assert b.list() == []


async def test_a_corrupt_frame_is_an_event_not_an_exception(pair):
    """The spec's Assumptions: "a failed decrypt is a typed event delivered to the
    application, not an exception thrown through its update loop"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    failures = []
    b.on("DecryptFailed", failures.append)
    got = []
    b.on("MessageReceived", got.append)

    await a.send_message(chat_a.id, "this one is fine")
    corrupt = bytearray(wire.a.sent[-1].data)
    corrupt[-1] ^= 0x01
    await wire.b.deliver(chat_b.id, bytes(corrupt))

    assert len(failures) == 1 and failures[0].chat_id == chat_b.id
    assert [e.text for e in got] == ["this one is fine"], "a rejected frame was delivered"


async def test_a_decrypt_failure_event_carries_no_plaintext_or_ciphertext(pair):
    """Contract §3: ``DecryptFailed`` carries "the failure's shape - never the
    ciphertext, never a partial plaintext"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    failures = []
    b.on("DecryptFailed", failures.append)
    await a.send_message(chat_a.id, "a sentence nobody else should see")
    corrupt = bytearray(wire.a.sent[-1].data)
    corrupt[-1] ^= 0x01
    await wire.b.deliver(chat_b.id, bytes(corrupt))

    rendered = repr(vars(failures[0]))
    assert "nobody else should see" not in rendered
    assert bytes(corrupt).hex()[:32] not in rendered.lower()


# --- the layer ----------------------------------------------------------------


async def test_a_notify_layer_is_sent_as_soon_as_the_chat_is_ready(pair):
    """§7.3: "As soon as a new secret chat has been created, immediately after the
    secret key has been successfully exchanged"."""
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    assert chat_b.layer >= 73, "the peer's NotifyLayer did not raise the stored layer"
    assert a.status(chat_a.id).layer >= 73


# --- the exported surface (T027) ----------------------------------------------


def test_the_package_exports_exactly_the_contract():
    """contracts/public-api.md §6: the protocol internals are "tested directly; they
    are not a supported surface"."""
    import telethon_secret_chat as pkg

    assert set(pkg.__all__) == {
        "SecretChatManager",
        "SecretChat",
        "ChatState",
        "StorageBackend",
        "MemoryStorage",
        "FileStorage",
        "SecretChatError",
        "ParameterRejected",
        "ChatNotReady",
        "ChatClosed",
        "StorageRequired",
        "LayerUnsupported",
        "ResendUnsatisfiable",
        "ChatRequested",
        "ChatReady",
        "ChatClosedEvent",
        "MessageReceived",
        "MessageAcknowledged",
        "ServiceActionReceived",
        "DecryptFailed",
        "MEDIA_KINDS",
        "CAPTIONLESS_KINDS",
    }


def test_no_protocol_internal_is_reachable_from_the_package_root():
    import telethon_secret_chat as pkg

    for internal in ("crypto", "dh", "framing", "sequence", "rekey", "schema", "secret_tl"):
        assert internal not in pkg.__all__


def test_there_is_no_way_to_supply_a_key_or_skip_a_check():
    """contracts §6: "There is no 'trust me' flag, because the one thing this
    package sells is that the checks ran"."""
    import inspect

    from telethon_secret_chat import SecretChatManager

    forbidden = ("key", "skip", "trust", "unsafe", "insecure", "force", "layer")
    for name in ("__init__", "create", "accept", "send_message"):
        parameters = inspect.signature(getattr(SecretChatManager, name)).parameters
        assert not [p for p in parameters if any(f in p.lower() for f in forbidden)]


async def test_formatted_text_is_parsed_rather_than_left_a_coroutine():
    """`client._parse_message_text` is ASYNC, and not awaiting it sent nothing.

    The live interop run found this: `send_message` raised
    `TypeError: cannot unpack non-iterable coroutine object` against real Telethon,
    with `RuntimeWarning: coroutine '_parse_message_text' was never awaited`. Every
    unit test missed it for one reason - `FakeClient` does not define that attribute
    at all, so `_parse_text` took its `None` fallback and the real branch never ran.
    A double that lacks the thing under test cannot fail for it.

    So this double HAS the attribute, and is async exactly like Telethon's.
    """

    class _Parsing:
        async def _parse_message_text(self, text, parse_mode):
            return text.upper(), ["entity"]

    manager = SecretChatManager(_Parsing(), MemoryStorage())

    text, entities = await manager._parse_text("hello")

    assert text == "HELLO", "the parser's result was not awaited"
    assert entities == ["entity"]


async def test_a_client_without_the_private_parser_still_sends_plain_text():
    """The documented fallback, which is the branch the fakes were exercising."""

    class _Bare:
        pass

    manager = SecretChatManager(_Bare(), MemoryStorage())

    assert await manager._parse_text("hello") == ("hello", None)
