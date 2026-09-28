"""Uncertain sends: retries keep the original wire identity; no real accounts."""

import asyncio

import pytest

from telethon_secret_chat.errors import SendPending

from telethon_secret_chat import SecretChatManager, crypto, framing
from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.events import MessageReceived
from telethon_secret_chat.schema import secret_tl as tl

from .fake_client import FakeClient
from .helpers import SEND_TYPES, KEY, FlakyClient, ready_manager


async def test_restart_retries_uncertain_send_with_exact_original_wire_identity():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(SendPending):
        await manager.send_message(chat.id, "uncertain send")
    original = client.sent[-1]
    assert manager._storage.retained_out(chat.id)[0]["pending"]
    client.failing = False
    recovered = SecretChatManager(client, storage=manager._storage)
    await recovered.start()
    try:
        retry = client.sent[-1]
        assert (retry.random_id, retry.data) == (original.random_id, original.data)
        assert recovered._entity(chat.id).out_seq_no == 1
        assert not recovered._storage.retained_out(chat.id)[0]["pending"]
    finally:
        await recovered.stop()


async def test_delete_uncertain_message_never_retries_its_plaintext_first():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(SendPending):
        await manager.send_message(chat.id, "DELETE-ME-SYNTHETIC")
    random_id = client.sent[-1].random_id
    client.failing = False
    await manager.delete_messages(chat.id, [random_id])
    for request in client.sent[1:]:
        wrapper = framing.unwrap(crypto.decrypt_frame(KEY, request.data, chat.out_x))
        assert not getattr(wrapper.message, "message", "")
        assert isinstance(wrapper.message.action, tl.DecryptedMessageActionDeleteMessages)
    assert all(
        "DELETE-ME-SYNTHETIC".encode().hex() not in item["body"]
        for item in manager._storage.retained_out(chat.id)
    )


async def test_flush_uncertain_message_removes_content_even_if_rpc_stays_down():
    client = FlakyClient()
    manager, chat = ready_manager(client)
    with pytest.raises(SendPending):
        await manager.send_message(chat.id, "FLUSH-ME-SYNTHETIC")
    manager._history[chat.id] = [MessageReceived(chat.id, 1, 0, "history")]
    with pytest.raises(SendPending):
        await manager.flush_history(chat.id)
    assert manager.read_history(chat.id) == []
    assert all(
        "FLUSH-ME-SYNTHETIC".encode().hex() not in item["body"]
        for item in manager._storage.retained_out(chat.id)
    )


async def test_cancelled_send_remains_retryable():
    entered = asyncio.Event()

    class Blocking(FakeClient):
        block = True

        async def __call__(self, request):
            if self.block and isinstance(request, SEND_TYPES):
                self.sent.append(request)
                entered.set()
                await asyncio.Future()
            return await super().__call__(request)

    client = Blocking()
    manager, chat = ready_manager(client)
    task = asyncio.create_task(manager.send_message(chat.id, "cancelled RPC"))
    await asyncio.wait_for(entered.wait(), timeout=5)
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    original = client.sent[-1]
    client.block = False
    await manager.retry_pending(chat.id)
    assert client.sent[-1].data == original.data
    assert client.sent[-1].random_id == original.random_id
    assert chat.out_seq_no == 1
    assert not manager._inflight


async def test_concurrent_send_rpcs_do_not_overlap_or_reorder():
    class Yielding(FakeClient):
        active = 0
        maximum = 0

        async def __call__(self, request):
            if not isinstance(request, SEND_TYPES):
                return await super().__call__(request)
            self.active += 1
            self.maximum = max(self.maximum, self.active)
            try:
                await asyncio.sleep(0)
                return await super().__call__(request)
            finally:
                self.active -= 1

    client = Yielding()
    manager, chat = ready_manager(client)
    ids = await asyncio.gather(*(manager.send_message(chat.id, str(n)) for n in range(20)))
    assert client.maximum == 1
    assert len(set(ids)) == 20
    wrappers = [
        framing.unwrap(crypto.decrypt_frame(KEY, item.data, chat.out_x)) for item in client.sent
    ]
    assert [item.out_seq_no for item in wrappers] == list(range(1, 40, 2))
    assert [item.message.message for item in wrappers] == [str(n) for n in range(20)]


async def test_legacy_resend_without_original_wire_metadata_fails_closed():
    from telethon_secret_chat.errors import ResendUnsatisfiable

    manager, chat = ready_manager()
    wrapper = framing.wrap(
        tl.DecryptedMessage(random_id=123, ttl=0, message="legacy"),
        layer=73,
        in_seq_no=0,
        out_seq_no=1,
    )
    item = {"seq_no": 1, "body": bytes(wrapper).hex()}
    manager._storage.queue_out(chat.id, item)
    with pytest.raises(ResendUnsatisfiable):
        await manager._transmit(chat, item)
    assert chat.state is ChatState.CLOSED
    assert not any(isinstance(request, SEND_TYPES) for request in manager._client.sent)
    assert manager._storage.retained_out(chat.id) == []
