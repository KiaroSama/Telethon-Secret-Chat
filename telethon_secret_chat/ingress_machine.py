"""Persist the release's gap buffers as part of a recoverable batch pre-state."""

from telethon._updates.messagebox import ENTRY_ACCOUNT, ENTRY_SECRET, PossibleGap
from telethon.extensions import BinaryReader

from .ingress_store import IngressRefused


def key_of(key):
    if key is ENTRY_ACCOUNT:
        return "account"
    if key is ENTRY_SECRET:
        return "secret"
    if type(key) is int and key > 0:
        return key
    raise IngressRefused()


def entry_of(key):
    if key == "account":
        return ENTRY_ACCOUNT
    if key == "secret":
        return ENTRY_SECRET
    if type(key) is int and key > 0:
        return key
    raise IngressRefused()


def snapshot(box):
    return {
        "gaps": [
            [key_of(key), [bytes(update) for update in gap.updates]]
            for key, gap in box.possible_gaps.items()
        ],
        "diff": sorted((key_of(key) for key in box.getting_diff_for), key=str),
    }


def restore(box, state):
    from telethon._updates.messagebox import next_updates_deadline

    deadline = next_updates_deadline()
    for key, raw_updates in state.get("gaps", []):
        updates = []
        for raw in raw_updates:
            with BinaryReader(raw) as reader:
                updates.append(reader.tgread_object())
        box.possible_gaps[entry_of(key)] = PossibleGap(deadline, updates)
    box.getting_diff_for = {entry_of(key) for key in state.get("diff", [])}
