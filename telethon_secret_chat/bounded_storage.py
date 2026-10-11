"""Transactional finite custody over the application's existing storage backend."""

from __future__ import annotations

import json
import math
import time
from contextlib import contextmanager
from copy import deepcopy

from .bounded_types import ChatPaused
from .storage import StorageBackend

SETTING = "bounded_secret_chat_custody_v1"
REASONS = {"operator", "capacity", "age", "admission", "storage", "ingress", "deadline"}


def size_of(value):
    def encode(item):
        if isinstance(item, bytes):
            return {"bytes": item.hex()}
        raise ValueError("unsupported custody value")

    return len(json.dumps(value, default=encode, ensure_ascii=False).encode("utf-8"))


class CustodyStorage(StorageBackend):
    def __init__(self, backend, limits):
        if (
            type(backend).save_setting is StorageBackend.save_setting
            or type(backend).load_setting is StorageBackend.load_setting
        ):
            raise ValueError("bounded custody requires transactional persistent settings")
        self.backend, self.limits = backend, limits
        self.depth = 0
        self.touched = set()
        stored = backend.load_setting(SETTING)
        self.controls = {} if stored is None else stored
        if not isinstance(self.controls, dict) or len(self.controls) > limits.chats:
            raise ChatPaused()
        for key, value in self.controls.items():
            if not (
                isinstance(key, str)
                and key.isascii()
                and key.isdigit()
                and int(key) > 0
                and str(int(key)) == key
                and isinstance(value, dict)
                and set(value) == {"pause", "lost", "since", "relay", "ages"}
                and value["pause"] in REASONS | {None}
                and type(value["lost"]) is bool
                and isinstance(value["relay"], list)
                and (not value["lost"] or value["pause"] is not None)
                and (
                    value["since"] is None
                    or (
                        type(value["since"]) in (int, float)
                        and math.isfinite(value["since"])
                        and 0 < value["since"] <= time.time()
                    )
                )
                and len(value["relay"]) <= limits.records
                and isinstance(value["ages"], dict)
                and len(value["ages"]) <= limits.records
                and all(
                    type(t) in (int, float) and math.isfinite(t) and 0 < t <= time.time()
                    for t in value["ages"].values()
                )
            ):
                raise ChatPaused()
            from . import sequence

            known = set()
            for item in value["relay"]:
                try:
                    wrapper = sequence.unpack(item)
                    if item["seq_no"] != wrapper.out_seq_no // 2 or item["seq_no"] in known:
                        raise ValueError
                    known.add(item["seq_no"])
                except Exception:
                    raise ChatPaused() from None
            if value["since"] != min(value["ages"].values(), default=None):
                raise ChatPaused()
            record = self.backend.load(int(key)) or {}
            incoming = (
                self.backend.peek_in(int(key))
                + record.get("pending_deliveries", [])
                + value["relay"]
            )
            expected = {"in:" + str(item["seq_no"]) for item in incoming}
            expected.update(
                "out:" + str(item["seq_no"]) for item in self.backend.retained_out(int(key))
            )
            if set(value["ages"]) != expected:
                raise ChatPaused()
        if stored is None and any(self.backend.load(chat_id) for chat_id in self.backend.list()):
            # No silent migration resetting the age of existing protocol custody.
            raise ValueError(
                "bounded mode requires a fresh store or complete bounded custody metadata"
            )

    def control(self, chat_id):
        key = str(chat_id)
        if key not in self.controls:
            if len(self.controls) >= self.limits.chats:
                raise ChatPaused(chat_id=chat_id)
            self.controls[key] = dict(pause=None, lost=False, since=None, relay=[], ages={})
        return self.controls[key]

    def usage(self, chat_id):
        record = self.backend.load(chat_id) or {}
        out, incoming = self.backend.retained_out(chat_id), self.backend.peek_in(chat_id)
        relay = self.controls.get(str(chat_id), {}).get("relay", [])
        count = len(out) + len(incoming) + len(record.get("pending_deliveries", [])) + len(relay)
        return count, size_of([record, out, incoming, relay])

    def refresh_age(self, chat_id):
        control = self.control(chat_id)
        record = self.backend.load(chat_id) or {}
        keys = {"out:" + str(item["seq_no"]) for item in self.backend.retained_out(chat_id)}
        # Mailbox/gap/relay transfer keeps the same incoming identity and original age.
        incoming = (
            self.backend.peek_in(chat_id) + record.get("pending_deliveries", []) + control["relay"]
        )
        keys.update("in:" + str(item["seq_no"]) for item in incoming)
        ages = control["ages"]
        control["ages"] = {key: ages.get(key, time.time()) for key in keys}
        control["since"] = min(control["ages"].values(), default=None)

    def check(self, chat_id, *, reserve_records=0, reserve_bytes=0):
        control = self.control(chat_id)
        count, amount = self.usage(chat_id)
        if (
            count + reserve_records > self.limits.records
            or amount + reserve_bytes > self.limits.bytes
        ):
            raise ChatPaused(chat_id=chat_id)
        if (
            count
            and control["since"] is not None
            and time.time() - control["since"] >= self.limits.age_seconds
        ):
            raise ChatPaused(chat_id=chat_id)
        return count, amount

    @contextmanager
    def transaction(self):
        outer = self.depth == 0
        previous = deepcopy(self.controls) if outer else None
        if outer:
            self.touched = set()
        self.depth += 1
        try:
            with self.backend.transaction():
                yield self
                if outer:
                    for chat_id in self.touched:
                        self.refresh_age(chat_id)
                        control = self.control(chat_id)
                        # Pausing/acknowledging must preserve or reduce aged custody.
                        if not control["pause"]:
                            self.check(chat_id)
                    self.backend.save_setting(SETTING, self.controls)
        except BaseException:
            if outer:
                self.controls = previous
            raise
        finally:
            self.depth -= 1

    def pause(self, chat_id, reason, lost=False):
        with self.transaction():
            control = self.control(chat_id)
            control["pause"] = control["pause"] or reason
            control["lost"] = control["lost"] or lost

    def _commit(self, chat_id, record):
        with self.transaction():
            self.touched.add(chat_id)
            self.backend.save(record)

    def load(self, chat_id):
        return self.backend.load(chat_id)

    def list(self):
        return self.backend.list()

    def delete(self, chat_id):
        with self.transaction():
            self.backend.delete(chat_id)
            # Consumer obligations outlive protocol closure; only explicit ack settles them.
            control = self.controls.get(str(chat_id))
            if control and control["relay"]:
                control["pause"] = control["pause"] or "operator"
            else:
                self.controls.pop(str(chat_id), None)

    def queue_out(self, chat_id, message):
        with self.transaction():
            self.touched.add(chat_id)
            self.backend.queue_out(chat_id, message)

    def drop_out(self, chat_id, up_to_seq):
        with self.transaction():
            self.touched.add(chat_id)
            self.backend.drop_out(chat_id, up_to_seq)

    def retained_out(self, chat_id):
        return self.backend.retained_out(chat_id)

    def queue_in(self, chat_id, message):
        with self.transaction():
            self.touched.add(chat_id)
            self.backend.queue_in(chat_id, message)

    def peek_in(self, chat_id):
        return self.backend.peek_in(chat_id)

    def take_in(self, chat_id):
        with self.transaction():
            self.touched.add(chat_id)
            return self.backend.take_in(chat_id)

    def requeue_in(self, chat_id, messages):
        with self.transaction():
            self.touched.add(chat_id)
            self.backend.requeue_in(chat_id, messages)

    def load_setting(self, name):
        return self.backend.load_setting(name)

    def save_setting(self, name, value):
        with self.transaction():
            self.backend.save_setting(name, value)
