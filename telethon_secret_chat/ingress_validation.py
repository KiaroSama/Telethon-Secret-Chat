"""Validate protected checkpoints before they can authorize network acknowledgment."""

import math
import time

from telethon.extensions import BinaryReader

from .ingress_store import IngressRefused


def counter(value):
    return type(value) is int and 0 <= value < 2**63


def validate_checkpoint(state):
    if not isinstance(state, dict) or set(state) not in (
        {"session", "channels"},
        {"session", "channels", "machine"},
    ):
        raise IngressRefused()
    session = state["session"]
    if (
        not isinstance(session, dict)
        or set(session) != {"pts", "qts", "date", "seq"}
        or not all(counter(value) for value in session.values())
    ):
        raise IngressRefused()
    channels = state["channels"]
    if not isinstance(channels, list) or len(channels) > 4096:
        raise IngressRefused()
    ids = set()
    for item in channels:
        if (
            not isinstance(item, list)
            or len(item) != 2
            or not counter(item[0])
            or item[0] == 0
            or not counter(item[1])
            or item[0] in ids
        ):
            raise IngressRefused()
        ids.add(item[0])
    machine = state.get("machine", {"gaps": [], "diff": []})
    if not isinstance(machine, dict) or set(machine) != {"gaps", "diff"}:
        raise IngressRefused()
    gaps, diff = machine["gaps"], machine["diff"]
    if not isinstance(gaps, list) or not isinstance(diff, list) or len(gaps) + len(diff) > 4096:
        raise IngressRefused()
    keys = set()
    for key in diff:
        if not (key in ("account", "secret") or type(key) is int and key in ids):
            raise IngressRefused()
        if key in keys:
            raise IngressRefused()
        keys.add(key)
    keys = set()
    for item in gaps:
        if not isinstance(item, list) or len(item) != 2:
            raise IngressRefused()
        key, frames = item
        if (
            not (key in ("account", "secret") or type(key) is int and key in ids)
            or key in keys
            or not isinstance(frames, list)
            or len(frames) > 4096
        ):
            raise IngressRefused()
        keys.add(key)
        for raw in frames:
            if type(raw) is not bytes:
                raise IngressRefused()
            with BinaryReader(raw) as reader:
                reader.tgread_object()
                if reader.tell_position() != len(raw):
                    raise IngressRefused()


def validate_ledger(state, limits):
    if (
        not isinstance(state, dict)
        or set(state) != {"checkpoint", "batches", "paused"}
        or type(state["paused"]) is not bool
        or not isinstance(state["batches"], list)
    ):
        raise IngressRefused()
    if state["checkpoint"] is None:
        if state["batches"]:
            raise IngressRefused()
        return
    validate_checkpoint(state["checkpoint"])
    if len(state["batches"]) > limits.records:
        raise IngressRefused()
    identities = set()
    previous = None
    for item in state["batches"]:
        if (
            not isinstance(item, dict)
            or set(item)
            != {
                "identity",
                "kind",
                "envelope",
                "before",
                "after",
                "cursor",
                "applied",
                "created",
                "request",
            }
            or item["kind"] not in ("live", "difference")
            or type(item["envelope"]) is not bytes
            or len(item["envelope"]) > limits.batch_bytes
            or type(item["request"]) is not bytes
            or not counter(item["cursor"])
            or type(item["created"]) not in (int, float)
            or not math.isfinite(item["created"])
            or not 0 < item["created"] <= time.time()
            or type(item["identity"]) is not str
            or len(item["identity"]) != 64
            or any(c not in "0123456789abcdef" for c in item["identity"])
            or item["identity"] in identities
        ):
            raise IngressRefused()
        import hashlib

        digest = hashlib.sha256(
            item["kind"].encode("ascii") + item["request"] + item["envelope"]
        ).hexdigest()
        if digest != item["identity"]:
            raise IngressRefused()
        identities.add(digest)
        validate_checkpoint(item["before"])
        validate_checkpoint(item["after"])
        before, after = item["before"]["session"]["qts"], item["after"]["session"]["qts"]
        if before > after or after > state["checkpoint"]["session"]["qts"]:
            raise IngressRefused()
        if previous is not None and before != previous:
            raise IngressRefused()
        previous = after
        from .ingress_replay import updates_for
        from .ingress_types import IngressBatch

        batch = IngressBatch(
            digest, item["kind"], item["envelope"], item["before"], item["after"], item["cursor"]
        )
        total = len(updates_for(batch))
        applied = item["applied"]
        if (
            item["cursor"] > total
            or not isinstance(applied, list)
            or any(type(index) is not int or not 0 <= index < total for index in applied)
            or applied != sorted(set(applied))
        ):
            raise IngressRefused()
        contiguous = 0
        while contiguous in applied:
            contiguous += 1
        if contiguous != item["cursor"]:
            raise IngressRefused()
        from .ingress import DurableIngress, load_box
        from telethon._updates import MessageBox
        import logging

        box = MessageBox(logging.getLogger("telethon_secret_chat"))
        load_box(box, item["before"])
        request = None
        if item["request"]:
            with BinaryReader(item["request"]) as reader:
                request = reader.tgread_object()
                if reader.tell_position() != len(item["request"]):
                    raise IngressRefused()
        if (item["kind"] == "live") != (request is None):
            raise IngressRefused()
        with BinaryReader(item["envelope"]) as reader:
            envelope = reader.tgread_object()
            if reader.tell_position() != len(item["envelope"]):
                raise IngressRefused()
        preview = object.__new__(DurableIngress)
        preview.limits = limits
        _, _, projected, _ = preview._candidate(envelope, box, item["kind"], request)
        if projected != item["after"]:
            raise IngressRefused()
    if previous is not None and previous != state["checkpoint"]["session"]["qts"]:
        raise IngressRefused()
