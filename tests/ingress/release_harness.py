"""Real pinned Telethon processing with deterministic network/storage boundaries."""

import asyncio
import collections
import datetime
import logging

from telethon import events
from telethon.client.updates import UpdateMethods
from telethon._updates.messagebox import MessageBox, ENTRY_ACCOUNT
from telethon._updates.entitycache import EntityCache
from telethon._updates.session import SessionState
from telethon.tl import functions, types

UTC = datetime.timezone.utc
DATE = datetime.datetime(2026, 10, 11, tzinfo=UTC)


class DeletingServer:
    """The documented model: acknowledgment deletes events <= input frontier."""

    def __init__(self):
        self.owed = {41: b"public-fixture-envelope"}
        self.requests = []

    def acknowledge(self, request):
        self.requests.append(request)
        qts = (
            request.qts
            if isinstance(request, functions.updates.GetDifferenceRequest)
            else request.max_qts
        )
        self.owed = {seq: data for seq, data in self.owed.items() if seq > qts}


class RefusedCommit:
    async def process_entities(self, response):
        return None

    def __init__(self):
        self.committed = {}

    async def raw(self, update):
        raise OSError("synthetic durable ingress commit refusal")


class ReleaseLoop(UpdateMethods):
    """Borrow real release methods; all I/O is deterministic simulation."""

    def __init__(self, source):
        self._log = collections.defaultdict(
            lambda: logging.getLogger("release-ingress-diagnostic")
        )
        self._message_box = MessageBox(self._log["box"])
        self._message_box.load(
            SessionState(0, 0, False, 10, 40, int(DATE.timestamp()), 1, None), []
        )
        self._mb_entity_cache = EntityCache(self_id=123)
        self._self_id = 123
        self._updates_queue = asyncio.Queue()
        self._catch_up = False
        self._sequential_updates = True
        self._authorized = True
        self._entity_cache_limit = 100
        self._event_handler_tasks = set()
        self._event_builders = [(events.Raw(), self.on_raw)]
        self._durable_ingress = None
        self._conversations = {}
        self._megagroup_cache = {}
        self.session = RefusedCommit()
        self.session.dc_id = 2
        self.session.auth_key = type("AuthId", (), {"key_id": 456})()
        self.server = DeletingServer()
        self.connected = True
        self.source = source
        self.seen = []
        self.received = asyncio.Event()
        self.allow_ack = asyncio.Event()
        self.acknowledged = asyncio.Event()
        self.loop = asyncio.get_running_loop()
        self._loop = self.loop
        self.flood_sleep_threshold = 0
        self._flood_waited_requests = {}
        self._no_updates = False
        self._request_retries = 0
        self._raise_last_call_error = True

    def is_connected(self):
        return self.connected

    async def disconnect(self):
        self.connected = False

    async def on_raw(self, update):
        self.seen.append(getattr(update, "qts", None))
        self.received.set()
        await self.allow_ack.wait()
        self._message_box.try_begin_get_diff(ENTRY_ACCOUNT, "diagnostic explicit difference")
        await self.session.raw(update)

    async def __call__(self, request):
        from telethon.client.users import UserMethods

        owner = self

        class Sender:
            def send(self, request, ordered=False):
                return owner._server_response(request)

        return await UserMethods._call(self, Sender(), request)

    async def _server_response(self, request):
        if isinstance(request, functions.messages.ReceivedQueueRequest):
            self.server.acknowledge(request)
            return []
        assert isinstance(request, functions.updates.GetDifferenceRequest)
        if self.source == "difference" and not self.seen:
            self.server.requests.append(request)
            return types.updates.Difference(
                new_messages=[],
                new_encrypted_messages=[
                    types.EncryptedMessage(
                        random_id=11,
                        chat_id=7,
                        date=DATE,
                        bytes=b"public-fixture-envelope",
                        file=types.EncryptedFileEmpty(),
                    )
                ],
                other_updates=[],
                chats=[],
                users=[],
                state=types.updates.State(pts=10, qts=41, date=DATE, seq=1, unread_count=0),
            )
        self.server.acknowledge(request)
        self.connected = False
        self.acknowledged.set()
        return types.updates.DifferenceEmpty(date=DATE, seq=1)
