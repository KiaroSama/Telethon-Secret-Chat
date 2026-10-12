"""Protected contiguous ingress frontier, independent of consumer settlement."""

from __future__ import annotations

import asyncio
import copy
import datetime as dt
import hashlib
import logging
import time

from telethon._updates import MessageBox, EntityCache, SessionState, ChannelState
from telethon.tl import types, functions
from telethon.extensions import BinaryReader

from .ingress_store import EncryptedIngressStorage, IngressRefused
from .ingress_types import IngressLimits, IngressBatch
from .bounded_storage import size_of

SETTING = "durable_ingress_v1"
log = logging.getLogger("telethon_secret_chat")


def encode_state(state):
    session, channels = state
    values = dict(session)
    values["date"] = int(values["date"].timestamp())
    return {
        "session": values,
        "channels": [[item["channel_id"], item["pts"]] for item in channels],
    }


def decode_state(state):
    session = dict(state["session"])
    session["date"] = dt.datetime.fromtimestamp(session["date"], tz=dt.timezone.utc)
    return session, [dict(channel_id=channel, pts=pts) for channel, pts in state["channels"]]


def load_box(box, state):
    session, channels = decode_state(state)
    box.load(
        SessionState(
            0,
            0,
            False,
            session["pts"],
            session["qts"],
            int(session["date"].timestamp()),
            session["seq"],
            None,
        ),
        [ChannelState(item["channel_id"], item["pts"]) for item in channels],
    )
    from .ingress_machine import restore

    restore(box, state.get("machine", {}))


class DurableIngress:
    """Opt-in coordinator for the patched Telethon extension, attached pre-connect.

    Bootstrap is explicit and never reconstructs previously acknowledged history.
    The store's caller-supplied identity/generation must match this authorization.
    """

    api_version = 1

    def __init__(self, storage, *, limits: IngressLimits):
        if type(storage) is not EncryptedIngressStorage or type(limits) is not IngressLimits:
            raise IngressRefused()
        if limits.bytes > storage.max_bytes:
            raise IngressRefused()
        self.storage, self.limits = storage, limits
        storage._ingress_limits = limits
        storage._validate_aggregate()
        self._lock = asyncio.Lock()
        self._bound = False
        from typing import Any

        self._client: Any = None
        self._manager_owner = None
        self._epoch = 0
        self._issued: dict[asyncio.Task, tuple[int, tuple[bytes, ...]]] = {}
        self._stopped = False
        self._failed = False
        state = storage.load_setting(SETTING)
        self._state = (
            state if state is not None else dict(checkpoint=None, batches=[], paused=False)
        )
        from .ingress_validation import validate_ledger

        try:
            validate_ledger(self._state, limits)
        except Exception:
            raise IngressRefused() from None
        self._check_budget(self._state)

    def _check_budget(self, state):
        if len(state["batches"]) > self.limits.records or size_of(state) > self.limits.bytes:
            raise IngressRefused()
        if any(
            time.time() - item["created"] >= self.limits.age_seconds for item in state["batches"]
        ):
            raise IngressRefused()

    def _ready(self, *, paused=False):
        if (
            not self._bound
            or self._stopped
            or self._failed
            or (self._state["paused"] and not paused)
        ):
            raise IngressRefused()
        self.storage._check_owner()
        self._check_budget(self._state)
        binding = self.storage.binding
        if (
            self._client.session.dc_id != binding["dc_id"]
            or getattr(getattr(self._client.session, "auth_key", None), "key_id", None)
            != binding["auth_key_id"]
        ):
            raise IngressRefused()
        known = getattr(self._client._mb_entity_cache, "self_id", None)
        if known is not None and known != binding["user_id"]:
            raise IngressRefused()
        self.storage._validate_aggregate()

    def _commit(self, state):
        self._check_budget(state)
        with self.storage.transaction():
            self.storage.save_setting(SETTING, state)
        self._state = state

    async def prepare(self, client, *, baseline=None):
        """Attach to the same caller client before connection; bootstrap explicitly.

        A fresh store requires a supplied verified MessageBox session-state tuple.
        No GetState RPC guesses history and no live unmodified client is converted.
        """
        from .ingress_provenance import verify_prerequisite

        verify_prerequisite()
        configure = getattr(client, "configure_durable_ingress", None)
        if not callable(configure) or client.is_connected():
            raise IngressRefused()
        await self.bind(client)
        if self._state["checkpoint"] is None:
            if baseline is None:
                raise IngressRefused()
            await self.bootstrap(baseline)
        elif baseline is not None:
            raise IngressRefused()
        configure(self)
        return self

    def validate_attachment(self, client):
        if self._client is not client or not self._bound or self._state["checkpoint"] is None:
            raise IngressRefused()
        self._ready()

    async def bind(self, client):
        if self._client is not None and self._client is not client:
            raise IngressRefused()
        if self._stopped:
            raise IngressRefused()
        binding = self.storage.binding
        key = getattr(getattr(client.session, "auth_key", None), "key_id", None)
        if client.session.dc_id != binding["dc_id"] or key != binding["auth_key_id"]:
            raise IngressRefused()
        known_user = getattr(client._mb_entity_cache, "self_id", None)
        if known_user is not None and known_user != binding["user_id"]:
            raise IngressRefused()
        self._client = client
        self._bound = True

    async def bootstrap(self, state):
        async with self._lock:
            self._ready()
            if self._state["checkpoint"] is not None or self._state["batches"]:
                raise IngressRefused()
            candidate = copy.deepcopy(self._state)
            candidate["checkpoint"] = encode_state(state)
            from .ingress_validation import validate_checkpoint

            validate_checkpoint(candidate["checkpoint"])
            self._commit(candidate)

    async def restore(self, box):
        self._ready(paused=True)
        checkpoint = self._state["checkpoint"]
        if checkpoint is None:
            raise IngressRefused()  # Caller must explicitly establish the initial baseline.
        load_box(box, checkpoint)
        return True

    def checkpoint(self):
        if self._state["checkpoint"] is None:
            raise IngressRefused()
        return copy.deepcopy(self._state["checkpoint"]["session"])

    def pending_batches(self):
        return [
            IngressBatch(
                item["identity"],
                item["kind"],
                item["envelope"],
                copy.deepcopy(item["before"]),
                copy.deepcopy(item["after"]),
                item["cursor"],
                tuple(item["applied"]),
            )
            for item in self._state["batches"]
        ]

    def _candidate(self, envelope, box, kind, request=None):
        raw = bytes(envelope)
        if len(raw) > self.limits.batch_bytes:
            raise IngressRefused()
        from .ingress_machine import snapshot

        before = encode_state(box.session_state())
        before["machine"] = snapshot(box)
        trial = MessageBox(log)
        # Retain the release's buffered gaps/deadlines as well as scalar counters.
        trial.map = {key: copy.deepcopy(value) for key, value in box.map.items()}
        trial.date, trial.seq = box.date, box.seq
        trial.next_deadline = box.next_deadline
        trial.possible_gaps = {
            key: copy.deepcopy(value) for key, value in box.possible_gaps.items()
        }
        trial.getting_diff_for = set(box.getting_diff_for)
        cache = EntityCache()
        if kind == "live":
            # Work on a detached TL object: release code adds transient fields.
            with BinaryReader(raw) as reader:
                detached = reader.tgread_object()
            processed: list = []
            from telethon._updates import GapError

            try:
                trial.process_updates(detached, cache, processed)
            except GapError:
                pass  # Exact envelope is durable; frontier must wait for the missing range.
        else:
            from telethon._updates.messagebox import ENTRY_ACCOUNT

            trial.try_begin_get_diff(ENTRY_ACCOUNT, "durable difference preview")
            trial.apply_difference(envelope, cache)
        after = encode_state(trial.session_state())
        after["machine"] = snapshot(trial)
        material = kind.encode("ascii") + (bytes(request) if request is not None else b"") + raw
        identity = hashlib.sha256(material).hexdigest()
        return raw, before, after, identity

    async def _capture(self, envelope, box, kind, request=None):
        async with self._lock:
            self._ready()
            if self._state["checkpoint"] is None:
                raise IngressRefused()
            try:
                raw, before, after, identity = self._candidate(envelope, box, kind, request)
                if any(item["identity"] == identity for item in self._state["batches"]):
                    return
                frontier = self.checkpoint()["qts"]
                if before["session"]["qts"] != frontier or after["session"]["qts"] < frontier:
                    raise IngressRefused()
                candidate = copy.deepcopy(self._state)
                candidate["batches"].append(
                    dict(
                        identity=identity,
                        kind=kind,
                        envelope=raw,
                        before=before,
                        after=after,
                        cursor=0,
                        applied=[],
                        created=time.time(),
                        request=bytes(request) if request is not None else b"",
                    )
                )
                candidate["checkpoint"] = after
                self._commit(candidate)
            except Exception:
                self._failed = True
                log.warning("durable ingress refused; authorization acknowledgment stopped")
                raise IngressRefused() from None

    async def capture_live(self, envelope, box):
        await self._capture(envelope, box, "live")

    async def capture_difference(self, request, response, box):
        identity = hashlib.sha256(b"difference" + bytes(request) + bytes(response)).hexdigest()
        receipt = getattr(response, "_tsc_ingress_receipt", None)
        if receipt == (id(self), self._epoch, identity):
            self._ready()
            return  # RPC capture remains proven even after its pending batch was drained.
        if any(item["identity"] == identity for item in self._state["batches"]):
            self._ready()
            return
        if request.qts != box.session_state()[0]["qts"]:
            raise IngressRefused()
        if isinstance(response, types.updates.DifferenceTooLong):
            self._failed = True
            raise IngressRefused()
        await self._capture(response, box, "difference", request)
        response._tsc_ingress_receipt = id(self), self._epoch, identity

    async def guard_requests(self, requests):
        async with self._lock:
            self._ready()
            if self._state["checkpoint"] is None:
                raise IngressRefused()
            frontier = self.checkpoint()["qts"]
            from .ingress_requests import unwrap, frontier_of

            for request in requests:
                request = unwrap(request)
                acknowledged = frontier_of(request)
                if acknowledged is not None and (
                    type(acknowledged) is not int or not 0 <= acknowledged <= frontier
                ):
                    raise IngressRefused()
                if isinstance(request, functions.updates.GetStateRequest):
                    raise IngressRefused()  # Bootstrap cannot silently reset the durable baseline.
            task = asyncio.current_task()
            if task is None:
                raise IngressRefused()
            if task not in self._issued:
                task.add_done_callback(lambda finished: self._issued.pop(finished, None))
            self._issued[task] = self._epoch, tuple(bytes(unwrap(item)) for item in requests)

    async def guard_result(self, request, response, box):
        from .ingress_requests import unwrap

        request = unwrap(request)
        self._ready()
        task = asyncio.current_task()
        if task is None:
            raise IngressRefused()
        issued = self._issued.get(task)
        if issued is None or issued[0] != self._epoch or bytes(request) not in issued[1]:
            raise IngressRefused()
        if isinstance(request, functions.updates.GetDifferenceRequest):
            await self.capture_difference(request, response, box)
            # The automatic loop applies its own response next. Caller-issued
            # Difference must not leave observed qts behind its committed custody.
            if task is not getattr(self._client, "_updates_handle", None):
                load_box(box, self._state["checkpoint"])
        if isinstance(request, functions.updates.GetStateRequest):
            raise IngressRefused()

    async def session_state(self, observed, channels):
        # Final session projection remains available after stop has fenced requests.
        if not self._bound or self._state["checkpoint"] is None:
            raise IngressRefused()
        self.storage._check_owner()
        return decode_state(self._state["checkpoint"])

    async def stop(self):
        self._stopped = True
        self._epoch += 1
