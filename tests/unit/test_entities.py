"""Entities crossing from Telethon's API layer into the end-to-end schema (§7.3).

Telethon hands back API-layer objects whose constructor ids are not always the
end-to-end ones (blockquote) or have no end-to-end counterpart at all (mention
name). Serializing them as they come writes a message the peer cannot parse.
"""

from telethon.tl import types as api

from telethon_secret_chat import entities
from telethon_secret_chat.schema import secret_tl as tl

from .fake_client import establish


def test_a_blockquote_gets_the_end_to_end_constructor():
    (quote,) = entities.to_secret([api.MessageEntityBlockquote(offset=0, length=3)], layer=144)
    assert isinstance(quote, tl.MessageEntityBlockquote)
    assert quote.CONSTRUCTOR_ID == 0x20DF5D0


def test_an_entity_the_schema_lacks_is_dropped():
    assert entities.to_secret([api.MessageEntityMentionName(0, 3, user_id=1)], layer=144) is None


def test_an_entity_newer_than_the_peer_layer_is_dropped():
    underline = [api.MessageEntityUnderline(offset=0, length=3)]
    assert entities.to_secret(underline, layer=73) is None
    assert entities.to_secret(underline, layer=144)[0].CONSTRUCTOR_ID == 0x9C4E7E8B


def test_fields_beyond_offset_and_length_are_carried():
    (link,) = entities.to_secret(
        [api.MessageEntityTextUrl(offset=1, length=2, url="https://example.org")], layer=73
    )
    assert (link.offset, link.length, link.url) == (1, 2, "https://example.org")


def test_a_secret_schema_entity_passes_through():
    bold = tl.MessageEntityBold(offset=0, length=1)
    assert entities.to_secret([bold], layer=73) == [bold]


async def test_formatted_text_is_parsed_with_the_client_default_and_arrives_mapped(pair):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    await a.send_message(chat_a.id, "**bold** text")
    assert wire.a.parse_modes == [()]
    assert got[-1].text == "bold text"
    assert isinstance(got[-1].entities[0], tl.MessageEntityBold)
