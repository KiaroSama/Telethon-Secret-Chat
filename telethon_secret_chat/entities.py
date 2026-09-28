"""Message entities, carried from Telethon's API layer into the end-to-end schema.

Telethon's parser returns API-layer objects. Most share their constructor id with
the end-to-end schema, but not all: blockquote is `#20df5d0` here and `#f1ccaaac`
in the API, and mention-name, phone, cashtag and bank-card have no end-to-end form
at all. Serializing them as they come writes a message the peer cannot parse, which
its client treats as a permanent gap.

§7.3 in the other direction: announcing a layer implies parsing what it introduced,
so an entity newer than the peer's layer is not sent either. Unmappable entities are
dropped, never refused - the text still arrives, only its formatting is lost.
"""

from __future__ import annotations

from .schema import secret_tl as tl

__all__ = ["to_secret"]

# protocol-reference.md §7.1: 101 added underline/strike/blockquote, 144 spoiler and
# custom emoji. Everything else in the schema predates the MTProto 2.0 floor of 73.
MINIMUM_LAYER = {
    "MessageEntityUnderline": 101,
    "MessageEntityStrike": 101,
    "MessageEntityBlockquote": 101,
    "MessageEntitySpoiler": 144,
    "MessageEntityCustomEmoji": 144,
}


def to_secret(entities, *, layer: int):
    """The entities the peer at ``layer`` can parse, as end-to-end objects, or None."""
    kept = []
    for entity in entities or ():
        name = type(entity).__name__
        target = getattr(tl, name, None)
        if target is None or not name.startswith("MessageEntity"):
            continue
        if MINIMUM_LAYER.get(name, 0) > layer:
            continue
        if isinstance(entity, tl.SecretTLObject):
            kept.append(entity)
            continue
        fields = target.__init__.__code__.co_varnames[1 : target.__init__.__code__.co_argcount]
        kept.append(target(**{field: getattr(entity, field, None) for field in fields}))
    return kept or None
