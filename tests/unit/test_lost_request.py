"""A request whose answer was lost (plan 027).

The server created the chat but ``requestEncryption``'s response never reached
``create()``, so this side kept no secret. Observed live on 2026-09-29: a chat made by
ANOTHER device of the same account reaches this session only as
``encryptedChatDiscarded``, never waiting or ready. So an unknown waiting or ready
chat outside a ``create()`` in flight can only be our own lost request, and it is
discarded on the server rather than left for the peer to accept into a dead chat.
"""

from telethon.tl import functions, types


def _waiting(chat_id):
    return types.EncryptedChatWaiting(
        id=chat_id, access_hash=1, date=0, admin_id=1000, participant_id=2000
    )


def _ready(chat_id):
    return types.EncryptedChat(
        id=chat_id,
        access_hash=1,
        date=0,
        admin_id=1000,
        participant_id=2000,
        g_a_or_b=(3).to_bytes(256, "big"),
        key_fingerprint=1,
    )


def _discarded(wire):
    return [
        r.chat_id for r in wire.sent if isinstance(r, functions.messages.DiscardEncryptionRequest)
    ]


async def test_an_unknown_waiting_chat_is_a_lost_request_and_is_discarded(pair):
    wire, a, _ = pair
    failed = []
    a.on("DecryptFailed", failed.append)
    await wire.a.deliver_update(types.UpdateEncryption(chat=_waiting(4242), date=0))
    assert _discarded(wire.a) == [4242]
    assert [(f.chat_id, f.reason) for f in failed] == [(4242, "request lost; discarded")]


async def test_an_unknown_ready_chat_outside_create_is_discarded(pair):
    wire, a, _ = pair
    await wire.a.deliver_update(types.UpdateEncryption(chat=_ready(4343), date=0))
    assert _discarded(wire.a) == [4343]


async def test_an_unknown_chat_while_create_is_in_flight_is_kept(pair):
    wire, a, _ = pair
    a._creating = 1
    try:
        await wire.a.deliver_update(types.UpdateEncryption(chat=_waiting(4444), date=0))
        await wire.a.deliver_update(types.UpdateEncryption(chat=_ready(4444), date=0))
    finally:
        a._creating = 0
    assert _discarded(wire.a) == []
    assert 4444 in a._early_encryption


async def test_our_own_waiting_chat_is_left_alone(pair):
    wire, a, _ = pair
    chat = await a.create(2000)
    await wire.a.deliver_update(types.UpdateEncryption(chat=_waiting(chat.id), date=0))
    assert _discarded(wire.a) == []
