"""A Telethon client stand-in, and a pair of managers wired back to back.

Not a mock of the package: a mock of TELEGRAM. It answers the four requests the
handshake and the send path make, records what was sent, and - in ``Wire`` - hands
one manager's ciphertext to the other exactly as the server would.

That last part is what makes the unit tier worth having. Two managers talking
through this wire exercise both sides of every asymmetry in §1.6 - the crypto ``x``,
the ``seq_no`` parity, which end computes the fingerprint and which compares it -
and a side confusion shows up as a rejection rather than as a chat that works in
tests and not in life.

It is NOT evidence of protocol correctness: both ends are this package, which is the
"two instances of the same wrong code" Principle I names. That evidence is
tests/vectors (Telethon's primitives) and tests/interop (a real client).
"""

from __future__ import annotations

import asyncio

from telethon.tl import functions, types

from .dh_material import SAFE_PRIME

G = 2


class FakeClient:
    """Answers the requests ``manager.py`` makes, and remembers them."""

    def __init__(self, user_id=1000):
        self.user_id = user_id
        self.sent = []  # every request passed to __call__
        self.handlers = []
        self._next_chat_id = 500
        self.peer = None  # a Wire sets this to the other FakeClient
        # "The server sent something else." An override rather than a patched
        # __call__, because Python looks __call__ up on the TYPE - assigning it to
        # an instance changes nothing, which is a quietly passing test.
        self.dh_prime_override = None
        # While held, an encrypted message is kept rather than delivered. That is
        # what the real server does across the handshake: A processes the
        # updateEncryption carrying the key before any message that follows it.
        self.hold = False
        self.held = []
        # Every blob handed to the server, so a test can assert the plaintext never
        # was one (§6.4: the bytes are encrypted client-side BEFORE upload).
        self.uploaded = []
        self.sent_files = []  # the file handle of every sendEncryptedFile, in order
        self.stored = {}  # file id -> ciphertext, standing in for the CDN
        # file id -> the fingerprint given at upload: the server keeps it with the file,
        # so a forward by `inputEncryptedFile` (which carries none) still reports it.
        self.fingerprints = {}
        # A request with no branch below. It also raises, but a raise inside an update
        # handler becomes a silent DecryptFailed, so teardown checks this list too.
        self.unanswered = []
        # Telethon's private parser, as async as the real one. Records the parse mode it
        # was asked for, so a test can pin the client-default sentinel `()`.
        self.parse_modes = []
        self.download_targets = []
        # Deferred: each handler runs as its own task, as Telethon dispatches updates
        # concurrently, instead of inline inside the sender's RPC.
        self.deferred = False
        self.tasks = []

    # --- the surface manager.py uses -----------------------------------------

    async def __call__(self, request):
        self.sent.append(request)
        if isinstance(request, functions.messages.GetDhConfigRequest):
            prime = self.dh_prime_override or SAFE_PRIME
            return types.messages.DhConfig(
                g=G, p=prime.to_bytes(256, "big"), version=1, random=b""
            )
        if isinstance(request, functions.messages.RequestEncryptionRequest):
            self._next_chat_id += 1
            return types.EncryptedChatWaiting(
                id=self._next_chat_id,
                access_hash=7777,
                date=0,
                admin_id=self.user_id,
                participant_id=999,
            )
        if isinstance(request, functions.messages.AcceptEncryptionRequest):
            return types.EncryptedChat(
                id=request.peer.chat_id,
                access_hash=request.peer.access_hash,
                date=0,
                admin_id=999,
                participant_id=self.user_id,
                g_a_or_b=request.g_b,
                key_fingerprint=request.key_fingerprint,
            )
        if isinstance(
            request,
            (
                functions.messages.SendEncryptedRequest,
                functions.messages.SendEncryptedServiceRequest,
            ),
        ):
            if self.hold:
                self.held.append((request.peer.chat_id, request.data))
            elif self.peer is not None:
                await self.peer.deliver(request.peer.chat_id, request.data)
            return types.messages.SentEncryptedMessage(date=0)
        if isinstance(request, functions.messages.SendEncryptedFileRequest):
            given = getattr(request.file, "key_fingerprint", None)
            if given is not None:
                self.fingerprints[request.file.id] = given
                if self.peer is not None:
                    self.peer.fingerprints[request.file.id] = given
            self.sent_files.append(request.file)
            if self.hold:
                self.held.append((request.peer.chat_id, request.data, request.file))
            elif self.peer is not None:
                await self.peer.deliver(request.peer.chat_id, request.data, request.file)
            # The server answers a file send with the stored file's handle, which the
            # outbox keeps for resends (messages.sentEncryptedFile).
            return types.messages.SentEncryptedFile(
                date=0,
                file=types.EncryptedFile(
                    id=request.file.id,
                    access_hash=0,
                    size=len(self.stored.get(request.file.id, b"")),
                    dc_id=1,
                    key_fingerprint=self.fingerprints.get(request.file.id, 0),
                ),
            )
        if isinstance(request, functions.messages.DiscardEncryptionRequest):
            if self.peer is not None and not self.hold:
                await self.peer.deliver_update(
                    types.UpdateEncryption(
                        chat=types.EncryptedChatDiscarded(id=request.chat_id), date=0
                    )
                )
            return True  # Telethon maps boolTrue to Python's True; types has no BoolTrue.
        self.unanswered.append(type(request).__name__)
        raise AssertionError(f"the manager sent a request this fake does not answer: {request!r}")

    def assert_quiet(self):
        if self.unanswered:
            raise AssertionError(f"unanswered requests: {self.unanswered}")

    async def _parse_message_text(self, message, parse_mode):
        from telethon.tl import types as api

        self.parse_modes.append(parse_mode)
        if parse_mode != () or "**" not in message:
            return message, []
        start = message.index("**")
        plain = message.replace("**", "", 2)
        length = message.index("**", start + 2) - start - 2
        return plain, [api.MessageEntityBold(offset=start, length=length)]

    async def upload_file(self, file, **kwargs):
        """Telethon's public upload. The bytes arriving here are ciphertext."""
        size = kwargs.get("file_size")
        if size is not None:
            # Telethon's contract for a stream: read(part) must return exactly `part`
            # bytes on every part but the last (client/uploads.py, 1.45.0).
            part, pieces = 512 * 1024, []
            while sum(map(len, pieces)) < size:
                piece = file.read(part)
                assert piece and (len(piece) == part or sum(map(len, pieces)) + len(piece) == size)
                pieces.append(piece)
            blob = b"".join(pieces)
        else:
            blob = file.read() if hasattr(file, "read") else bytes(file)
        self.uploaded.append(blob)
        file_id = len(self.uploaded)
        # Both sides of a Wire share one store, as one Telegram CDN would.
        self.stored[file_id] = blob
        if self.peer is not None:
            self.peer.stored[file_id] = blob
        return types.InputFile(id=file_id, parts=1, name="upload.bin", md5_checksum="")

    async def download_file(self, location, out, **kwargs):
        self.download_targets.append(type(out).__name__)
        out.write(self.stored[location.id])
        return out

    async def get_input_entity(self, user):
        return types.InputUser(user_id=int(user), access_hash=0)

    def add_event_handler(self, callback, event=None):
        self.handlers.append(callback)

    def remove_event_handler(self, callback, event=None):
        # Identity, deliberately stricter than Telethon's own `==`. A manager that
        # unsubscribes with a freshly-built bound method passes under `==` and
        # leaves the handler installed under `is`; requiring identity here means the
        # manager cannot depend on which one its host happens to use.
        self.handlers = [h for h in self.handlers if h is not callback]

    # --- delivering an update -------------------------------------------------

    async def deliver(self, chat_id, data, file=None):
        """Hand an encrypted message to whatever is subscribed, as the server does.

        ``file`` is the ``encryptedFile`` the server returns for a media message -
        the address and the §6.2 fingerprint, in cleartext, OUTSIDE the encryption.
        """
        attached = types.EncryptedFileEmpty()
        if file is not None:
            attached = types.EncryptedFile(
                id=file.id,
                access_hash=0,
                size=len(self.stored.get(file.id, b"")),
                dc_id=1,
                key_fingerprint=getattr(
                    file, "key_fingerprint", self.fingerprints.get(file.id, 0)
                ),
            )
        update = types.UpdateNewEncryptedMessage(
            message=types.EncryptedMessage(
                random_id=1, chat_id=chat_id, date=0, bytes=data, file=attached
            ),
            qts=0,
        )
        await self.deliver_update(update)

    async def deliver_update(self, update):
        for handler in list(self.handlers):
            if self.deferred:
                self.tasks.append(asyncio.ensure_future(handler(update)))
            else:
                await handler(update)

    async def release(self):
        """Deliver everything held, in the order it was sent."""
        held, self.held, self.hold = self.held, [], False
        for item in held:
            await self.peer.deliver(*item)


class Wire:
    """Two fake clients whose encrypted traffic reaches each other."""

    def __init__(self):
        self.a = FakeClient(user_id=1000)
        self.b = FakeClient(user_id=2000)
        self.a.peer, self.b.peer = self.b, self.a

    def defer(self):
        """From now on, every delivered update runs as a concurrent task."""
        self.a.deferred = self.b.deferred = True

    async def settle(self):
        """Await delivery tasks, and the ones they start, until none remain."""
        for _ in range(100):
            pending = [task for task in self.a.tasks + self.b.tasks if not task.done()]
            if not pending:
                break
            await asyncio.wait_for(asyncio.gather(*pending), 5)
        else:
            raise AssertionError("delivery never settled")
        for task in self.a.tasks + self.b.tasks:
            task.result()  # re-raise a handler failure here, not as a warning


async def establish(manager_a, manager_b, wire):
    """Drive a full §1.3-§1.6 exchange between two managers. Returns both chats.

    The update sequence is the documented one: A requests, B is told through
    ``encryptedChatRequested``, B accepts, and A learns the result through
    ``encryptedChat`` carrying ``g_a_or_b`` and ``key_fingerprint`` (§1.4).
    """
    chat_a = await manager_a.create(2000)

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
                folder_id=None,
            ),
            date=0,
        )
    )
    # B's own NotifyLayer (§7.3) goes out the instant B has the key, which is
    # before A has processed the update carrying it. The server holds it; so does
    # this fake, and it is released once A is keyed.
    wire.b.hold = True
    chat_b = await manager_b.accept(chat_a.id)

    accepted = [
        r for r in wire.b.sent if isinstance(r, functions.messages.AcceptEncryptionRequest)
    ]
    await wire.a.deliver_update(
        types.UpdateEncryption(
            chat=types.EncryptedChat(
                id=chat_a.id,
                access_hash=7777,
                date=0,
                admin_id=1000,
                participant_id=2000,
                g_a_or_b=accepted[-1].g_b,
                key_fingerprint=accepted[-1].key_fingerprint,
            ),
            date=0,
        )
    )
    await wire.b.release()
    return manager_a._entity(chat_a.id), manager_b._entity(chat_b.id)
