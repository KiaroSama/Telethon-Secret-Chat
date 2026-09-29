"""plans/025: a server REJECTION of a retained send is not a network failure.

Telegram's method pages (messages.sendEncrypted, sendEncryptedFile,
sendEncryptedService) list the errors; the package sorts them into transient,
permanent and chat-ending. No real accounts.
"""

import pytest
from telethon import errors
from telethon.tl import functions

from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.errors import ChatClosed, SendPending
from telethon_secret_chat.schema import secret_tl as tl

from .helpers import SEND_TYPES, FlakyClient, ready_manager


class RejectingClient(FlakyClient):
    """Rejects sends of the chosen request types with the chosen Telegram error."""

    def __init__(self, error, types=SEND_TYPES, times=None):
        super().__init__()
        self.failing = False
        self.error, self.types, self.times = error, types, times

    async def __call__(self, request):
        if isinstance(request, self.types) and self.times != 0:
            if self.times is not None:
                self.times -= 1
            self.sent.append(request)
            raise self.error(request=request)
        return await super().__call__(request)


async def test_a_permanently_rejected_message_becomes_a_self_delete():
    client = RejectingClient(errors.DataInvalidError, times=1)
    manager, chat = ready_manager(client)
    failed = []
    manager.on("SendFailed", failed.append)

    with pytest.raises(SendPending) as caught:
        await manager.send_message(chat.id, "never deliverable")
    random_id = caught.value.random_id

    assert [(event.random_id, event.cause) for event in failed] == [
        (random_id, "DataInvalidError")
    ]
    record = manager._storage.retained_out(chat.id)[0]
    assert record["method"] == "service" and record["random_id"] == random_id

    # The slot now carries a §3.8 self-delete; the next send clears it first.
    await manager.send_message(chat.id, "next")
    assert not any(item["pending"] for item in manager._storage.retained_out(chat.id))
    assert chat.state is ChatState.READY


async def test_a_protocol_step_rejected_twice_ends_the_chat():
    client = RejectingClient(
        errors.DataInvalidError, types=(functions.messages.SendEncryptedServiceRequest,)
    )
    manager, chat = ready_manager(client)

    with pytest.raises(SendPending):
        await manager.set_ttl(chat.id, 5)
    assert chat.state is ChatState.READY
    with pytest.raises(ChatClosed):
        await manager.retry_pending(chat.id)
    assert chat.state is ChatState.CLOSED


async def test_a_chat_the_server_calls_gone_closes_locally():
    client = RejectingClient(errors.EncryptionDeclinedError)
    manager, chat = ready_manager(client)
    closed = []
    manager.on("ChatClosed", closed.append)

    with pytest.raises(ChatClosed):
        await manager.send_message(chat.id, "too late")
    assert chat.state is ChatState.CLOSED and len(closed) == 1
    assert not any(
        isinstance(request, functions.messages.DiscardEncryptionRequest) for request in client.sent
    )


async def test_a_stuck_record_does_not_hold_back_the_ones_behind_it():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(SendPending) as caught:
        await manager.send_message(chat.id, "stuck")
    stuck = caught.value.random_id

    # Commit a second record behind it without the retry that normally runs first.
    async def skip(chat_id):
        return None

    manager.retry_pending = skip
    with pytest.raises(SendPending):
        await manager._send(
            chat,
            tl.DecryptedMessageService(random_id=77, action=tl.DecryptedMessageActionNoop()),
        )
    del manager.retry_pending

    class OnlyStuckFails(FlakyClient):
        async def __call__(self, request):
            self.failing = getattr(request, "random_id", None) == stuck
            return await super().__call__(request)

    manager._client.__class__ = OnlyStuckFails
    with pytest.raises(SendPending):
        await manager.retry_pending(chat.id)
    pending = {
        item["random_id"] for item in manager._storage.retained_out(chat.id) if item["pending"]
    }
    assert pending == {stuck}
