"""Long random lifecycles: start, stop, restart, send, TTL, close and storage faults.

The 2026-09-30 re-audit found lifecycle defects that only showed in particular
orderings (a stale upload writing after a restart, a TTL applied to memory but not to
storage, a failed shutdown leaving a manager half-stopped). Hand-written tests cover
the orderings someone thought of; this machine generates the others and checks,
after EVERY step, what must hold in all of them:

- every chat a running manager holds is stored exactly as it is in memory;
- a closed chat holds no key material, in memory or in its stored record;
- a manager is subscribed to updates exactly when it is running and not stopping.

Pattern: the machine owns its event loop and runs each rule to completion on it
(Hypothesis has no async rules). Delivery is inline, as the fake wire does it.
"""

import asyncio

from hypothesis import HealthCheck, settings, strategies as st
from hypothesis.stateful import RuleBasedStateMachine, initialize, invariant, precondition, rule

from telethon_secret_chat import SecretChatManager
from telethon_secret_chat.chat import ChatState
from telethon_secret_chat.errors import SecretChatError
from telethon_secret_chat.storage import MemoryStorage

from .fake_client import Wire, establish


class InjectedFault(OSError):
    """A write the storage refuses, raised once when armed."""


class FaultyStorage(MemoryStorage):
    def __init__(self):
        super().__init__()
        self.fail_next = False

    def _fail_if_armed(self):
        if self.fail_next:
            self.fail_next = False
            raise InjectedFault("injected storage failure")

    def _commit(self, chat_id, record):
        # Also an unchanged record: stop() saves every chat, changed or not.
        self._fail_if_armed()
        super()._commit(chat_id, record)

    def _write(self):
        self._fail_if_armed()


EXPECTED = (SecretChatError, InjectedFault)
SIDES = st.sampled_from("ab")


class Lifecycle(RuleBasedStateMachine):
    def __init__(self):
        super().__init__()
        self.loop = asyncio.new_event_loop()
        self.wire = Wire()
        self.stores = {"a": FaultyStorage(), "b": FaultyStorage()}
        self.clients = {"a": self.wire.a, "b": self.wire.b}
        self.managers = {
            side: SecretChatManager(self.clients[side], storage=self.stores[side]) for side in "ab"
        }
        self.chat_ids = []

    def run(self, awaitable):
        return self.loop.run_until_complete(awaitable)

    def attempt(self, awaitable):
        """Run one operation; the documented failures are legal outcomes. True when
        it returned normally."""
        try:
            self.run(awaitable)
        except EXPECTED:
            return False
        return True

    def running(self, side):
        manager = self.managers[side]
        return manager._running and not manager._stopping

    @initialize()
    def established(self):
        a, b = self.managers["a"], self.managers["b"]
        self.run(a.start())
        self.run(b.start())
        chat_a, _ = self.run(establish(a, b, self.wire))
        self.chat_ids.append(chat_a.id)

    @precondition(lambda self: self.running("a") and self.running("b") and len(self.chat_ids) < 3)
    @rule()
    def open_another(self):
        try:
            chat_a, _ = self.run(establish(self.managers["a"], self.managers["b"], self.wire))
        except EXPECTED:
            return
        self.chat_ids.append(chat_a.id)

    @rule(side=SIDES, index=st.integers(0, 2), text=st.text(max_size=20))
    def send(self, side, index, text):
        self.attempt(
            self.managers[side].send_message(self.chat_ids[index % len(self.chat_ids)], text)
        )

    @rule(side=SIDES, index=st.integers(0, 2), seconds=st.integers(0, 3600))
    def set_ttl(self, side, index, seconds):
        self.attempt(
            self.managers[side].set_ttl(self.chat_ids[index % len(self.chat_ids)], seconds)
        )

    @rule(side=SIDES, index=st.integers(0, 2))
    def close(self, side, index):
        self.attempt(self.managers[side].close(self.chat_ids[index % len(self.chat_ids)]))

    @rule(side=SIDES)
    def stop(self, side):
        if self.attempt(self.managers[side].stop()):
            assert not self.managers[side]._running

    @rule(side=SIDES)
    def start(self, side):
        # A start that returns normally has a running, subscribed manager - also
        # after a shutdown whose save failed (the re-audit's F10 restart case).
        if self.attempt(self.managers[side].start()):
            assert self.running(side)

    @rule(side=SIDES)
    def fail_next_write(self, side):
        self.stores[side].fail_next = True

    @invariant()
    def memory_matches_storage(self):
        for side, manager in self.managers.items():
            if not self.running(side):
                continue
            for chat in manager._chats.values():
                # A closed chat keeps a scrubbed record until forget().
                assert self.stores[side].load(chat.id) == chat.to_record()
                if chat.state is ChatState.CLOSED:
                    assert not chat.holds_material()

    @invariant()
    def subscribed_exactly_while_running(self):
        for side, manager in self.managers.items():
            assert bool(self.clients[side].handlers) == self.running(side)

    def teardown(self):
        try:
            for side in "ab":
                self.stores[side].fail_next = False
                manager = self.managers[side]
                if manager._running or manager._stopping:
                    self.run(manager.stop())
                    self.run(manager.stop())  # a second stop is a no-op
            self.run(self.loop.shutdown_asyncgens())
        finally:
            self.loop.close()


class TestLifecycle(Lifecycle.TestCase):
    # derandomize: CI runs the same sequences every time; a failure is reproducible.
    settings = settings(
        max_examples=100,
        stateful_step_count=25,
        deadline=None,
        derandomize=True,
        suppress_health_check=[HealthCheck.too_slow],
    )
