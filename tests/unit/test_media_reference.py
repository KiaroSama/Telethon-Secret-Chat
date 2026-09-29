"""A media reference keeps a received file reachable after a restart (spec 005, US2).

It holds the file's one-time key, so it is key material for that one file: where it is kept is
the application's choice (docs/design/cutover-history.md option A), and nothing the package prints,
logs or raises ever shows the key or iv (Principle IV).
"""

import json
import logging
import os

import pytest

from telethon_secret_chat import MediaReference
from telethon_secret_chat.errors import MessageRejected

from .fake_client import establish


@pytest.fixture
async def received(pair, tmp_path):
    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    content = os.urandom(5000)
    source = tmp_path / "photo.jpg"
    source.write_bytes(content)
    await a.send_file(chat_a.id, source, caption="look")
    await a.send_message(chat_a.id, "text only")
    return wire, a, b, got, content


async def test_only_a_message_with_a_file_carries_a_reference(received):
    _, _, _, got, _ = received
    with_file, text_only = got
    assert isinstance(with_file.media_reference, MediaReference)
    assert text_only.media_reference is None


async def test_a_reference_round_trips_through_plain_data(received):
    _, _, _, got, _ = received
    ref = got[0].media_reference
    data = json.loads(json.dumps(ref.to_dict()))
    assert MediaReference.from_dict(data) == ref
    assert ref.size == 5000 and ref.mime_type == "image/jpeg"


async def test_saving_from_a_reference_writes_the_same_bytes(received, tmp_path):
    _, _, b, got, content = received
    ref = MediaReference.from_dict(got[0].media_reference.to_dict())
    written = await b.save_file(ref, tmp_path / "from-ref.jpg")
    assert written.read_bytes() == content


async def test_a_reference_whose_key_does_not_match_is_refused_before_writing(received, tmp_path):
    _, _, b, got, _ = received
    ref = got[0].media_reference
    wrong = MediaReference(**{**vars(ref), "key_fingerprint": ref.key_fingerprint ^ 1})
    target = tmp_path / "never.jpg"
    with pytest.raises(MessageRejected):
        await b.save_file(wrong, target)
    assert not target.exists()


@pytest.mark.parametrize(
    "change",
    [
        {"version": 2},
        {"chat_id": "1"},
        {"file_id": None},
        {"media": "zz"},
        {"media": "00"},
    ],
)
async def test_a_damaged_dict_is_refused_naming_the_field_only(received, change):
    _, _, _, got, _ = received
    data = dict(got[0].media_reference.to_dict(), **change)
    with pytest.raises(ValueError) as caught:
        MediaReference.from_dict(data)
    assert got[0].media_reference.to_dict()["media"][:32] not in str(caught.value)


async def test_no_repr_log_or_error_shows_the_key(received, tmp_path, caplog):
    _, _, b, got, _ = received
    ref = got[0].media_reference
    media = ref.decoded()
    secrets_hex = {media.key.hex(), media.iv.hex()}
    caplog.set_level(logging.DEBUG)
    rendered = [repr(ref), str(ref), repr(got[0])]
    wrong = MediaReference(**{**vars(ref), "key_fingerprint": ref.key_fingerprint ^ 1})
    with pytest.raises(MessageRejected) as caught:
        await b.save_file(wrong, tmp_path / "x")
    rendered += [str(caught.value), repr(caught.value), caplog.text]
    for text in rendered:
        for secret in secrets_hex:
            assert secret not in text
            assert secret[:16] not in text
