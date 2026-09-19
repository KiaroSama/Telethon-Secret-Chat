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
        if isinstance(request, functions.messages.SendEncryptedRequest):
            if self.hold:
                self.held.append((request.peer.chat_id, request.data))
            elif self.peer is not None:
                await self.peer.deliver(request.peer.chat_id, request.data)
            return types.messages.SentEncryptedMessage(date=0)
        if isinstance(request, functions.messages.DiscardEncryptionRequest):
            return types.BoolTrue()
        raise AssertionError(f"the manager sent a request this fake does not answer: {request!r}")

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

    async def deliver(self, chat_id, data):
        """Hand an encrypted message to whatever is subscribed, as the server does."""
        update = types.UpdateNewEncryptedMessage(
            message=types.EncryptedMessage(
                random_id=1, chat_id=chat_id, date=0, bytes=data, file=types.EncryptedFileEmpty()
            ),
            qts=0,
        )
        for handler in list(self.handlers):
            await handler(update)

    async def deliver_update(self, update):
        for handler in list(self.handlers):
            await handler(update)

    async def release(self):
        """Deliver everything held, in the order it was sent."""
        held, self.held, self.hold = self.held, [], False
        for chat_id, data in held:
            await self.peer.deliver(chat_id, data)


class Wire:
    """Two fake clients whose encrypted traffic reaches each other."""

    def __init__(self):
        self.a = FakeClient(user_id=1000)
        self.b = FakeClient(user_id=2000)
        self.a.peer, self.b.peer = self.b, self.a


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
    return manager_a.status(chat_a.id), chat_b
