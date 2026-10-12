"""Decode committed batches using their saved pre-state, never fabricated qts."""

import copy
import logging

from telethon._updates import MessageBox, EntityCache
from telethon._updates.messagebox import ENTRY_ACCOUNT
from telethon.extensions import BinaryReader
from telethon.tl import types

from .ingress_store import IngressRefused


def updates_for(batch):
    from .ingress import load_box

    with BinaryReader(batch.envelope) as reader:
        envelope = reader.tgread_object()
    box = MessageBox(logging.getLogger("telethon_secret_chat"))
    load_box(box, batch.before)
    cache = EntityCache()
    if batch.kind == "difference":
        box.try_begin_get_diff(ENTRY_ACCOUNT, "committed ingress replay")
        updates, _, _ = box.apply_difference(envelope, cache)
    elif batch.kind == "live":
        from telethon._updates import GapError

        updates = []
        try:
            box.process_updates(envelope, cache, updates)
        except GapError:
            pass  # No event was applied; the saved machine waits for missing ingress.
    else:
        raise IngressRefused()
    return updates


def is_secret_update(update):
    return isinstance(update, (types.UpdateEncryption, types.UpdateNewEncryptedMessage))


def advance(coordinator, identity, cursor):
    """Called inside the SAME backend transaction as protocol application."""
    coordinator._ready(paused=True)
    candidate = copy.deepcopy(coordinator._state)
    for item in candidate["batches"]:
        if item["identity"] == identity:
            if type(cursor) is not int or cursor <= 0:
                raise IngressRefused()
            applied = set(item["applied"])
            applied.add(cursor - 1)
            item["applied"] = sorted(applied)
            contiguous = item["cursor"]
            while contiguous in applied:
                contiguous += 1
            item["cursor"] = contiguous
            coordinator.storage.save_setting("durable_ingress_v1", candidate)
            return candidate
    raise IngressRefused()
