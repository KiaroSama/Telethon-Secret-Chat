"""The generated schema against the page it was generated from.

Two different jobs here, and the first is the one that matters. A constructor id is
a CRC32 the peer uses to decide what it just received: get one wrong and the
message is not malformed, it is a DIFFERENT message, or nothing at all. So the
first test re-reads `end-to-end.tl` - the file fetched verbatim from
https://core.telegram.org/schema/end-to-end - and asserts the module agrees with it
constructor for constructor. Regenerating from a mangled schema fails here.

The second job is the serializer: flags, vectors, nested objects and TL's byte
padding, round-tripped. protocol-reference.md §3.1 for the wrapper, §5 for the
actions, §6 for media.
"""

import re
from pathlib import Path

import pytest

from telethon.extensions import BinaryReader

from telethon_secret_chat.schema import secret_tl as tl

SCHEMA_TL = Path(tl.__file__).with_name("end-to-end.tl")


def published():
    """Every ``name#id`` on the fetched page, as the page states it."""
    out = {}
    for line in SCHEMA_TL.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line.endswith(";") or line.startswith("//"):
            continue
        m = re.match(r"(\w+)#([0-9a-f]{1,8})", line)
        if m:
            out[int(m.group(2), 16)] = m.group(1)
    return out


def roundtrip(obj):
    """Bytes out, object back, through the module's own registry."""
    with BinaryReader(bytes(obj)) as reader:
        return tl.read_object(reader)


# --- the schema is the published one ------------------------------------------


def test_every_published_constructor_is_registered():
    missing = {f"{name}#{cid:x}" for cid, name in published().items() if cid not in tl.REGISTRY}
    assert not missing, f"constructors on the page but not in the module: {sorted(missing)}"


def test_no_constructor_was_invented():
    extra = {cid for cid in tl.REGISTRY if cid not in published()}
    assert not extra, f"constructors in the module but not on the page: {extra}"


def test_the_ids_the_protocol_reference_quotes_are_the_ones_generated():
    """Fifteen ids appear in prose in docs/protocol-reference.md. They are quoted
    there from the same source, so agreement is a real cross-check on the fetch and
    on the parser - not a restatement of one file by itself."""
    quoted = {
        0x1BE31789: "decryptedMessageLayer",  # §3.1
        0x73164160: "decryptedMessageService",  # §3.1
        0xAA48327D: "decryptedMessageService",  # §3.1, the layer-8 shape
        0xA1733AEC: "decryptedMessageActionSetMessageTTL",  # §5.1
        0x0C4F40BE: "decryptedMessageActionReadMessages",  # §5.2
        0x65614304: "decryptedMessageActionDeleteMessages",  # §5.3
        0x8AC1F475: "decryptedMessageActionScreenshotMessages",  # §5.4
        0x6719E45C: "decryptedMessageActionFlushHistory",  # §5.5
        0x511110B0: "decryptedMessageActionResend",  # §5.6
        0xF3048883: "decryptedMessageActionNotifyLayer",  # §5.7
        0xCCB27641: "decryptedMessageActionTyping",  # §5.8
        0xF3C9611B: "decryptedMessageActionRequestKey",  # §5.9, §4.2
        0x6FE1735B: "decryptedMessageActionAcceptKey",  # §5.10, §4.3
        0xEC2E0B9B: "decryptedMessageActionCommitKey",  # §5.11, §4.4
        0xDD05EC6B: "decryptedMessageActionAbortKey",  # §5.12, §4.6
        0xA82FDD63: "decryptedMessageActionNoop",  # §5.13
    }
    for cid, name in quoted.items():
        assert cid in tl.REGISTRY, f"{name}#{cid:08x} is quoted in the reference and absent"
        assert tl.REGISTRY[cid].TL_NAME == name


def test_all_thirteen_actions_exist():
    """§5: thirteen constructors, no more. The set is closed."""
    actions = {
        c.TL_NAME for c in tl.REGISTRY.values() if c.RESULT_TYPE == "DecryptedMessageAction"
    }
    assert len(actions) == 13, sorted(actions)


# --- the serializer -----------------------------------------------------------


def test_a_plain_field_object_round_trips():
    obj = tl.DecryptedMessageActionSetMessageTTL(ttl_seconds=3600)
    assert roundtrip(obj) == obj


def test_a_vector_of_longs_round_trips():
    obj = tl.DecryptedMessageActionReadMessages(random_ids=[1, -2, 2**62])
    assert roundtrip(obj).random_ids == [1, -2, 2**62]


def test_an_empty_constructor_round_trips():
    assert roundtrip(tl.DecryptedMessageActionNoop()) == tl.DecryptedMessageActionNoop()


def test_bytes_fields_survive_their_padding():
    """TL pads a byte string to a multiple of four. Every length near the boundary,
    including the 254-byte long form, has to come back exactly."""
    for length in (0, 1, 2, 3, 4, 7, 253, 254, 255, 260):
        blob = bytes(range(256)) * 2
        obj = tl.DecryptedMessageActionRequestKey(exchange_id=7, g_a=blob[:length])
        assert roundtrip(obj).g_a == blob[:length], length


def test_a_string_field_round_trips_as_utf8():
    obj = tl.DecryptedMessage(random_id=1, ttl=0, message="ünïcödé ✓ 日本語")
    assert roundtrip(obj).message == "ünïcödé ✓ 日本語"


def test_an_absent_flag_field_stays_absent():
    obj = tl.DecryptedMessage(random_id=1, ttl=0, message="hi")
    back = roundtrip(obj)
    assert back.media is None and back.entities is None and back.grouped_id is None


def test_a_present_flag_field_comes_back():
    obj = tl.DecryptedMessage(
        random_id=1, ttl=0, message="hi", grouped_id=99, reply_to_random_id=5
    )
    back = roundtrip(obj)
    assert back.grouped_id == 99 and back.reply_to_random_id == 5


def test_a_true_flag_is_carried_by_its_bit_alone():
    """``no_webpage:flags.1?true`` occupies a bit and no bytes."""
    plain = tl.DecryptedMessage(random_id=1, ttl=0, message="x")
    flagged = tl.DecryptedMessage(random_id=1, ttl=0, message="x", no_webpage=True)
    assert len(bytes(flagged)) == len(bytes(plain))
    assert roundtrip(flagged).no_webpage is True
    assert roundtrip(plain).no_webpage is False


def test_a_nested_object_round_trips():
    """§3.1: every message travels inside decryptedMessageLayer."""
    inner = tl.DecryptedMessageService(
        random_id=42, action=tl.DecryptedMessageActionNotifyLayer(layer=144)
    )
    obj = tl.DecryptedMessageLayer(
        random_bytes=b"\x01" * 31, layer=144, in_seq_no=2, out_seq_no=3, message=inner
    )
    back = roundtrip(obj)
    assert back.layer == 144 and back.in_seq_no == 2 and back.out_seq_no == 3
    assert back.message.action.layer == 144
    assert back.random_bytes == b"\x01" * 31


def test_a_vector_of_objects_round_trips():
    obj = tl.DecryptedMessage(
        random_id=1,
        ttl=0,
        message="hi",
        entities=[tl.MessageEntityBold(offset=0, length=2)],
    )
    back = roundtrip(obj)
    assert len(back.entities) == 1 and back.entities[0].offset == 0


def test_the_layer_8_service_shape_is_accepted():
    """§3.1: a peer's NotifyLayer arrives in the obsolete wrapper and must parse."""
    obj = tl.DecryptedMessageService8(
        random_id=1, random_bytes=b"\x02" * 15, action=tl.DecryptedMessageActionNoop()
    )
    assert roundtrip(obj).random_id == 1


# --- refusals -----------------------------------------------------------------


def test_an_unknown_constructor_is_reported_not_guessed():
    """FR-011. A parser that guesses at an unrecognised shape is an oracle."""
    with pytest.raises(tl.UnknownConstructor):
        with BinaryReader(b"\xde\xad\xbe\xef") as reader:
            tl.read_object(reader)


def test_repr_carries_no_values():
    """Principle IV. §8.8 recorded the archived package logging these objects -
    which are plaintext message bodies - at DEBUG."""
    obj = tl.DecryptedMessage(random_id=1, ttl=0, message="the quick brown fox")
    assert "quick brown fox" not in repr(obj)
    assert "quick brown fox" not in str(obj)
