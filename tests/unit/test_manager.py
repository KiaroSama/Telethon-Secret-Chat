"""The README's API reference - the surface an application actually uses.

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

from .fake_client import establish

# --- establishment ------------------------------------------------------------


async def test_creating_a_chat_leaves_it_awaiting_acceptance(pair):
    """ "The application reports it as awaiting acceptance"."""
    wire, a, _ = pair
    chat = await a.create(2000)
    assert chat.state is ChatState.REQUESTED and chat.key_fingerprint is None


async def test_the_dh_configuration_is_validated_before_a_chat_is_requested(pair):
    """A chat established on a bad prime cannot be repaired afterwards."""
    wire, a, _ = pair
    wire.a.dh_prime_override = 2**2047 + 1  # right length, divisible by three
    with pytest.raises(ParameterRejected):
        await a.create(2000)
    assert a.list() == [], "a chat was recorded for parameters that were refused"
    assert not [
        r for r in wire.a.sent if isinstance(r, functions.messages.RequestEncryptionRequest)
    ], "a chat was REQUESTED on parameters that were refused"


async def test_both_ends_report_the_same_fingerprint(pair):
    """The one check no amount of unit testing replaces at the
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
    """``ChatRequested`` - "an incoming request awaiting accept"."""
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
    assert a._entity(chat.id).state is ChatState.CLOSED


# --- the conversation ---------------------------------------------------------


async def test_a_message_crosses_and_arrives_as_text(pair):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    await a.send_message(chat_a.id, "hello there")
    assert [e.text for e in got] == ["hello there"]


async def test_messages_arrive_in_order(pair):
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
    """``send_message`` resolves when Telegram accepts the ciphertext and returns
    the sent message's id."""
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
    """``close`` on an already closed chat does nothing and does not raise."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    await a.close(chat_a.id)
    await a.close(chat_a.id)
    assert a._entity(chat_a.id).state is ChatState.CLOSED


async def test_an_unknown_chat_is_refused_by_status_and_history(pair):
    wire, a, _ = pair
    with pytest.raises(KeyError):
        a._entity(999999)
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
    assert a._entity(chat_a.id).layer >= 73


# --- the exported surface ----------------------------------------------


def test_the_package_exports_exactly_the_contract():
    """The protocol internals are tested directly; they are not a supported surface."""
    import telethon_secret_chat as pkg

    assert set(pkg.__all__) == {
        "SecretChatManager",
        "SecretChat",
        "ChatSnapshot",
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
        "StoreCorrupt",
        "UnknownChat",
        "ManagerStopping",
        "SendPending",
        "ChatRequested",
        "ChatReady",
        "ChatClosedEvent",
        "MessageReceived",
        "MessageAcknowledged",
        "ServiceActionReceived",
        "DecryptFailed",
        "SendFailed",
        "MEDIA_KINDS",
        "CAPTIONLESS_KINDS",
        "MediaReference",
        "KeyVisualization",
        "key_visualization",
        "PALETTE",
    }


def test_no_protocol_internal_is_reachable_from_the_package_root():
    import telethon_secret_chat as pkg

    for internal in ("crypto", "dh", "framing", "sequence", "rekey", "schema", "secret_tl"):
        assert internal not in pkg.__all__


def test_there_is_no_way_to_supply_a_key_or_skip_a_check():
    """ "There is no 'trust me' flag, because the one thing this
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


# --- the server's side effects the fake now reproduces -------------------------


async def test_a_peer_discarding_the_chat_closes_it_here(pair):
    """Edge case "the peer discards the chat": reported closed, with one event."""
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    closed = []
    b.on("ChatClosedEvent", closed.append)
    await a.close(chat_a.id)
    assert b._entity(chat_a.id).state is ChatState.CLOSED
    assert b._entity(chat_a.id).closed_reason == "the peer discarded the chat"
    assert [event.chat_id for event in closed] == [chat_a.id]


async def test_a_sent_file_keeps_the_server_handle_for_resends(pair, tmp_path):
    """§3.7 answers a resend with the original bytes; for a file that means the
    server's handle, not a second upload."""
    from telethon.extensions import BinaryReader
    from telethon.tl import types

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    source = tmp_path / "note.txt"
    source.write_bytes(b"kept for resend")
    await a.send_file(chat_a.id, source)
    record = a._storage.retained_out(chat_a.id)[-1]
    with BinaryReader(bytes.fromhex(record["file"])) as reader:
        assert isinstance(reader.tgread_object(), types.InputEncryptedFile)


# --- start/stop under concurrent updates, handshake edges -----------


async def test_a_request_arriving_during_start_does_not_break_recovery():
    """Telethon dispatches updates concurrently; one landing while start() awaits
    recovery work adds a chat to the dict start() is walking."""
    from telethon.tl import types

    from .helpers import FlakyClient, ready_manager

    class Interrupting(FlakyClient):
        failing = False
        fired = False

        async def __call__(self, request):
            if isinstance(request, functions.messages.SendEncryptedRequest) and not self.fired:
                self.fired = True
                await self.deliver_update(
                    types.UpdateEncryption(
                        chat=types.EncryptedChatRequested(
                            id=4242,
                            access_hash=1,
                            date=0,
                            admin_id=5,
                            participant_id=1000,
                            g_a=(2**2047 + 12345).to_bytes(256, "big"),
                        ),
                        date=0,
                    )
                )
            return await super().__call__(request)

    first, chat = ready_manager(Interrupting())
    first._storage.queue_out(
        chat.id,
        {"seq_no": 1, "random_id": 1, "pending": True, "frame": "00" * 64, "method": "message"},
    )
    manager = SecretChatManager(Interrupting(), storage=first._storage)
    failed = []
    manager.on("DecryptFailed", failed.append)
    await manager.start()
    assert not failed, [f.reason for f in failed]
    try:
        assert 4242 in {c.id for c in manager.list()}, [
            type(r).__name__ for r in manager._client.sent
        ]
    finally:
        await manager.stop()


async def test_an_unsafe_incoming_request_is_discarded_on_the_server(pair):
    from telethon.tl import types

    wire, _, b = pair
    failed = []
    b.on("DecryptFailed", failed.append)
    await wire.b.deliver_update(
        types.UpdateEncryption(
            chat=types.EncryptedChatRequested(
                id=777,
                access_hash=1,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a=(1).to_bytes(256, "big"),
            ),
            date=0,
        )
    )
    assert failed and failed[0].chat_id == 777
    discards = [
        r for r in wire.b.sent if isinstance(r, functions.messages.DiscardEncryptionRequest)
    ]
    assert [r.chat_id for r in discards] == [777]


async def test_a_pending_request_accepted_elsewhere_closes_locally_only(pair):
    from telethon.tl import types

    wire, a, b = pair
    chat_a = await a.create(2000)
    request = wire.a.sent[-1]
    await wire.b.deliver_update(
        types.UpdateEncryption(
            chat=types.EncryptedChatRequested(
                id=chat_a.id,
                access_hash=7777,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a=request.g_a,
            ),
            date=0,
        )
    )
    await wire.b.deliver_update(
        types.UpdateEncryption(
            chat=types.EncryptedChat(
                id=chat_a.id,
                access_hash=7777,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a_or_b=request.g_a,
                key_fingerprint=1,
            ),
            date=0,
        )
    )
    assert b._entity(chat_a.id).state is ChatState.CLOSED
    assert not any(isinstance(r, functions.messages.DiscardEncryptionRequest) for r in wire.b.sent)


async def test_an_async_handler_scheduled_while_stopping_is_not_left_pending():
    from telethon_secret_chat.events import ChatReady

    from .helpers import ready_manager

    manager, _ = ready_manager()
    ran = []

    async def handler(event):
        ran.append(event)

    manager.on("ChatReady", handler)
    manager._stopping = True
    manager._emit(ChatReady(1, 2, 3))
    assert not manager._handler_tasks and not ran


# --- one error family, and snapshots instead of the live entity -----


async def test_an_unknown_chat_is_a_package_error_and_still_a_key_error(pair):
    from telethon_secret_chat.errors import SecretChatError, UnknownChat

    _, a, _ = pair
    with pytest.raises(UnknownChat) as caught:
        a.status(999999)
    assert isinstance(caught.value, SecretChatError) and isinstance(caught.value, KeyError)


async def test_status_hands_out_a_read_only_snapshot_without_key_material(pair):
    import dataclasses

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    snapshot = a.status(chat_a.id)
    for name in ("key", "pending_key", "previous_key", "exchange_secret", "handshake"):
        assert not hasattr(snapshot, name), name
    assert not any(
        isinstance(value, bytes) and len(value) == 256 for value in vars(snapshot).values()
    )
    with pytest.raises(dataclasses.FrozenInstanceError):
        snapshot.ttl = 5
    live = a._chats[chat_a.id]
    assert (snapshot.key_fingerprint, snapshot.key_hash) == (live.key_fingerprint, live.key_hash)
    assert [c.id for c in a.list()] == [chat_a.id]


async def test_history_keeps_only_the_most_recent_messages():
    from .fake_client import Wire

    wire = Wire()
    a = SecretChatManager(wire.a, storage=MemoryStorage())
    b = SecretChatManager(wire.b, storage=MemoryStorage(), history_limit=5)
    await a.start()
    await b.start()
    try:
        chat_a, _ = await establish(a, b, wire)
        for n in range(8):
            await a.send_message(chat_a.id, f"m{n}")
        assert [m.text for m in b.read_history(chat_a.id, limit=50)] == [
            f"m{n}" for n in range(3, 8)
        ]
    finally:
        await a.stop()
        await b.stop()


def test_every_public_manager_method_is_documented():
    """``help(SecretChatManager)`` is the API reference."""
    import inspect

    assert inspect.getdoc(SecretChatManager)
    undocumented = [
        name
        for name, member in inspect.getmembers(SecretChatManager)
        if not name.startswith("_") and callable(member) and not inspect.getdoc(member)
    ]
    assert undocumented == []
