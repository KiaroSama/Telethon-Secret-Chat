# generated
# ruff: noqa
"""GENERATED from ``end-to-end.tl`` by ``tools/generate_schema.py``. Do not edit.

Telegram's end-to-end TL schema as Python. Every constructor id here is the one
published at https://core.telegram.org/schema/end-to-end; nothing in this file was
typed by hand, which is the point - protocol-reference.md quotes fifteen of these
ids and all fifteen are reproduced by the generator from the fetched page.

Exempt from the constitution's 800-line ceiling as generated code, and marked
``# generated`` on line 1 as the constitution requires.

Dispatch is by constructor id through ``REGISTRY``; Telethon's global
``alltlobjects`` registry is never read and never mutated.
"""

from __future__ import annotations

import struct

_VECTOR = b"\x15\xc4\xb5\x1c"
_BOOL_TRUE = b"\xb5\x75\x72\x99"
_BOOL_FALSE = b"\x37\x97\x79\xbc"


class UnknownConstructor(ValueError):
    """A constructor id this schema does not define.

    Raised rather than guessed. FR-011 wants an unknown action reported, not
    dropped, and a wrong guess at a shape is how a parser becomes an oracle.
    """

    def __init__(self, constructor_id: int):
        self.constructor_id = constructor_id
        super().__init__(f"unknown constructor 0x{constructor_id:08x}")


def _pack_int(value: int) -> bytes:
    return struct.pack("<i", value) if -(2**31) <= value < 2**31 else struct.pack("<I", value)


def _pack_long(value: int) -> bytes:
    return struct.pack("<q", value) if -(2**63) <= value < 2**63 else struct.pack("<Q", value)


def _pack_double(value: float) -> bytes:
    return struct.pack("<d", value)


def _serialize_bytes(data) -> bytes:
    """TL byte strings: a short length prefix, then padding to a multiple of four.

    The same rule Telethon's ``TLObject.serialize_bytes`` implements; written here
    so the generated module has no import from a Telethon private path.
    """
    if isinstance(data, str):
        data = data.encode("utf-8")
    elif not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("a TL string or bytes field takes str or bytes")
    data = bytes(data)
    if len(data) < 254:
        padding = (len(data) + 1) % 4
        if padding:
            padding = 4 - padding
        return bytes([len(data)]) + data + bytes(padding)
    padding = len(data) % 4
    if padding:
        padding = 4 - padding
    return b"\xfe" + len(data).to_bytes(3, "little") + data + bytes(padding)


def _read_vector(r, read_item):
    marker = r.read_int(signed=False)
    if marker != 0x1CB5C415:
        raise ValueError("expected a vector")
    return [read_item() for _ in range(r.read_int())]


def read_object(r):
    """Read one boxed object using THIS schema's registry."""
    constructor_id = r.read_int(signed=False)
    cls = REGISTRY.get(constructor_id)
    if cls is None:
        raise UnknownConstructor(constructor_id)
    return cls.from_reader(r)


class SecretTLObject:
    """Base for every constructor below."""

    CONSTRUCTOR_ID = 0
    TL_NAME = ""
    RESULT_TYPE = ""

    def __bytes__(self) -> bytes:
        w = [_pack_int(self.CONSTRUCTOR_ID)]
        self._write_body(w)
        return b"".join(w)

    def _write_body(self, w):
        raise NotImplementedError

    @classmethod
    def from_reader(cls, r):
        raise NotImplementedError

    def to_dict(self) -> dict:
        return {"_": self.TL_NAME, **{k: v for k, v in vars(self).items()}}

    def __eq__(self, other) -> bool:
        return type(self) is type(other) and vars(self) == vars(other)

    def __repr__(self) -> str:
        """Names only, never values.

        Constitution Principle IV: these objects carry plaintext message bodies
        and, in the rekey actions, public exchange values. A default repr would
        print them the first time one reached a log line or a traceback - which is
        exactly how the archived package leaked plaintext at DEBUG (§8.8).
        """
        return f"{type(self).__name__}(<{len(vars(self))} fields>)"


class DecryptedMessage_1f814f1f(SecretTLObject):
    """``decryptedMessage#1f814f1f`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0x1F814F1F
    TL_NAME = "decryptedMessage"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(self, random_id=None, random_bytes=None, message=None, media=None):
        self.random_id = random_id
        self.random_bytes = random_bytes
        self.message = message
        self.media = media

    def _write_body(self, w):
        w.append(_pack_long(self.random_id))
        w.append(_serialize_bytes(self.random_bytes))
        w.append(_serialize_bytes(self.message))
        w.append(bytes(self.media))

    @classmethod
    def from_reader(cls, r):
        random_id = r.read_long()
        random_bytes = r.tgread_bytes()
        message = r.tgread_string()
        media = read_object(r)
        return cls(random_id=random_id, random_bytes=random_bytes, message=message, media=media)


class DecryptedMessageService8(SecretTLObject):
    """``decryptedMessageService#aa48327d`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0xAA48327D
    TL_NAME = "decryptedMessageService"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(self, random_id=None, random_bytes=None, action=None):
        self.random_id = random_id
        self.random_bytes = random_bytes
        self.action = action

    def _write_body(self, w):
        w.append(_pack_long(self.random_id))
        w.append(_serialize_bytes(self.random_bytes))
        w.append(bytes(self.action))

    @classmethod
    def from_reader(cls, r):
        random_id = r.read_long()
        random_bytes = r.tgread_bytes()
        action = read_object(r)
        return cls(random_id=random_id, random_bytes=random_bytes, action=action)


class DecryptedMessageMediaEmpty(SecretTLObject):
    """``decryptedMessageMediaEmpty#89f5c4a`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x089F5C4A
    TL_NAME = "decryptedMessageMediaEmpty"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DecryptedMessageMediaPhoto_32798a8c(SecretTLObject):
    """``decryptedMessageMediaPhoto#32798a8c`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x32798A8C
    TL_NAME = "decryptedMessageMediaPhoto"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self, thumb=None, thumb_w=None, thumb_h=None, w=None, h=None, size=None, key=None, iv=None
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.w = w
        self.h = h
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(
            thumb=thumb, thumb_w=thumb_w, thumb_h=thumb_h, w=w, h=h, size=size, key=key, iv=iv
        )


class DecryptedMessageMediaVideo_4cee6ef3(SecretTLObject):
    """``decryptedMessageMediaVideo#4cee6ef3`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x4CEE6EF3
    TL_NAME = "decryptedMessageMediaVideo"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        duration=None,
        w=None,
        h=None,
        size=None,
        key=None,
        iv=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.duration = duration
        self.w = w
        self.h = h
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_pack_int(self.duration))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        duration = r.read_int()
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            duration=duration,
            w=w,
            h=h,
            size=size,
            key=key,
            iv=iv,
        )


class DecryptedMessageMediaGeoPoint(SecretTLObject):
    """``decryptedMessageMediaGeoPoint#35480a59`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x35480A59
    TL_NAME = "decryptedMessageMediaGeoPoint"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self, lat=None, long=None):
        self.lat = lat
        self.long = long

    def _write_body(self, w):
        w.append(_pack_double(self.lat))
        w.append(_pack_double(self.long))

    @classmethod
    def from_reader(cls, r):
        lat = r.read_double()
        long = r.read_double()
        return cls(lat=lat, long=long)


class DecryptedMessageMediaContact(SecretTLObject):
    """``decryptedMessageMediaContact#588a0a97`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x588A0A97
    TL_NAME = "decryptedMessageMediaContact"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self, phone_number=None, first_name=None, last_name=None, user_id=None):
        self.phone_number = phone_number
        self.first_name = first_name
        self.last_name = last_name
        self.user_id = user_id

    def _write_body(self, w):
        w.append(_serialize_bytes(self.phone_number))
        w.append(_serialize_bytes(self.first_name))
        w.append(_serialize_bytes(self.last_name))
        w.append(_pack_int(self.user_id))

    @classmethod
    def from_reader(cls, r):
        phone_number = r.tgread_string()
        first_name = r.tgread_string()
        last_name = r.tgread_string()
        user_id = r.read_int()
        return cls(
            phone_number=phone_number, first_name=first_name, last_name=last_name, user_id=user_id
        )


class DecryptedMessageActionSetMessageTTL(SecretTLObject):
    """``decryptedMessageActionSetMessageTTL#a1733aec`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xA1733AEC
    TL_NAME = "decryptedMessageActionSetMessageTTL"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, ttl_seconds=None):
        self.ttl_seconds = ttl_seconds

    def _write_body(self, w):
        w.append(_pack_int(self.ttl_seconds))

    @classmethod
    def from_reader(cls, r):
        ttl_seconds = r.read_int()
        return cls(ttl_seconds=ttl_seconds)


class DecryptedMessageMediaDocument_b095434b(SecretTLObject):
    """``decryptedMessageMediaDocument#b095434b`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0xB095434B
    TL_NAME = "decryptedMessageMediaDocument"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        file_name=None,
        mime_type=None,
        size=None,
        key=None,
        iv=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.file_name = file_name
        self.mime_type = mime_type
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_serialize_bytes(self.file_name))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        file_name = r.tgread_string()
        mime_type = r.tgread_string()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            file_name=file_name,
            mime_type=mime_type,
            size=size,
            key=key,
            iv=iv,
        )


class DecryptedMessageMediaAudio_6080758f(SecretTLObject):
    """``decryptedMessageMediaAudio#6080758f`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x6080758F
    TL_NAME = "decryptedMessageMediaAudio"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self, duration=None, size=None, key=None, iv=None):
        self.duration = duration
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_pack_int(self.duration))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        duration = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(duration=duration, size=size, key=key, iv=iv)


class DecryptedMessageActionReadMessages(SecretTLObject):
    """``decryptedMessageActionReadMessages#c4f40be`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x0C4F40BE
    TL_NAME = "decryptedMessageActionReadMessages"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, random_ids=None):
        self.random_ids = random_ids

    def _write_body(self, w):
        w.append(_VECTOR)
        w.append(_pack_int(len(self.random_ids)))
        for _item in self.random_ids:
            w.append(_pack_long(_item))

    @classmethod
    def from_reader(cls, r):
        random_ids = _read_vector(r, lambda: r.read_long())
        return cls(random_ids=random_ids)


class DecryptedMessageActionDeleteMessages(SecretTLObject):
    """``decryptedMessageActionDeleteMessages#65614304`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x65614304
    TL_NAME = "decryptedMessageActionDeleteMessages"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, random_ids=None):
        self.random_ids = random_ids

    def _write_body(self, w):
        w.append(_VECTOR)
        w.append(_pack_int(len(self.random_ids)))
        for _item in self.random_ids:
            w.append(_pack_long(_item))

    @classmethod
    def from_reader(cls, r):
        random_ids = _read_vector(r, lambda: r.read_long())
        return cls(random_ids=random_ids)


class DecryptedMessageActionScreenshotMessages(SecretTLObject):
    """``decryptedMessageActionScreenshotMessages#8ac1f475`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x8AC1F475
    TL_NAME = "decryptedMessageActionScreenshotMessages"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, random_ids=None):
        self.random_ids = random_ids

    def _write_body(self, w):
        w.append(_VECTOR)
        w.append(_pack_int(len(self.random_ids)))
        for _item in self.random_ids:
            w.append(_pack_long(_item))

    @classmethod
    def from_reader(cls, r):
        random_ids = _read_vector(r, lambda: r.read_long())
        return cls(random_ids=random_ids)


class DecryptedMessageActionFlushHistory(SecretTLObject):
    """``decryptedMessageActionFlushHistory#6719e45c`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x6719E45C
    TL_NAME = "decryptedMessageActionFlushHistory"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DecryptedMessage_204d3878(SecretTLObject):
    """``decryptedMessage#204d3878`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0x204D3878
    TL_NAME = "decryptedMessage"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(self, random_id=None, ttl=None, message=None, media=None):
        self.random_id = random_id
        self.ttl = ttl
        self.message = message
        self.media = media

    def _write_body(self, w):
        w.append(_pack_long(self.random_id))
        w.append(_pack_int(self.ttl))
        w.append(_serialize_bytes(self.message))
        w.append(bytes(self.media))

    @classmethod
    def from_reader(cls, r):
        random_id = r.read_long()
        ttl = r.read_int()
        message = r.tgread_string()
        media = read_object(r)
        return cls(random_id=random_id, ttl=ttl, message=message, media=media)


class DecryptedMessageService(SecretTLObject):
    """``decryptedMessageService#73164160`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0x73164160
    TL_NAME = "decryptedMessageService"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(self, random_id=None, action=None):
        self.random_id = random_id
        self.action = action

    def _write_body(self, w):
        w.append(_pack_long(self.random_id))
        w.append(bytes(self.action))

    @classmethod
    def from_reader(cls, r):
        random_id = r.read_long()
        action = read_object(r)
        return cls(random_id=random_id, action=action)


class DecryptedMessageMediaVideo_524a415d(SecretTLObject):
    """``decryptedMessageMediaVideo#524a415d`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x524A415D
    TL_NAME = "decryptedMessageMediaVideo"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        duration=None,
        mime_type=None,
        w=None,
        h=None,
        size=None,
        key=None,
        iv=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.duration = duration
        self.mime_type = mime_type
        self.w = w
        self.h = h
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_pack_int(self.duration))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        duration = r.read_int()
        mime_type = r.tgread_string()
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            duration=duration,
            mime_type=mime_type,
            w=w,
            h=h,
            size=size,
            key=key,
            iv=iv,
        )


class DecryptedMessageMediaAudio(SecretTLObject):
    """``decryptedMessageMediaAudio#57e0a9cb`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x57E0A9CB
    TL_NAME = "decryptedMessageMediaAudio"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self, duration=None, mime_type=None, size=None, key=None, iv=None):
        self.duration = duration
        self.mime_type = mime_type
        self.size = size
        self.key = key
        self.iv = iv

    def _write_body(self, w):
        w.append(_pack_int(self.duration))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))

    @classmethod
    def from_reader(cls, r):
        duration = r.read_int()
        mime_type = r.tgread_string()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        return cls(duration=duration, mime_type=mime_type, size=size, key=key, iv=iv)


class DecryptedMessageLayer(SecretTLObject):
    """``decryptedMessageLayer#1be31789`` -> ``DecryptedMessageLayer``."""

    CONSTRUCTOR_ID = 0x1BE31789
    TL_NAME = "decryptedMessageLayer"
    RESULT_TYPE = "DecryptedMessageLayer"

    def __init__(
        self, random_bytes=None, layer=None, in_seq_no=None, out_seq_no=None, message=None
    ):
        self.random_bytes = random_bytes
        self.layer = layer
        self.in_seq_no = in_seq_no
        self.out_seq_no = out_seq_no
        self.message = message

    def _write_body(self, w):
        w.append(_serialize_bytes(self.random_bytes))
        w.append(_pack_int(self.layer))
        w.append(_pack_int(self.in_seq_no))
        w.append(_pack_int(self.out_seq_no))
        w.append(bytes(self.message))

    @classmethod
    def from_reader(cls, r):
        random_bytes = r.tgread_bytes()
        layer = r.read_int()
        in_seq_no = r.read_int()
        out_seq_no = r.read_int()
        message = read_object(r)
        return cls(
            random_bytes=random_bytes,
            layer=layer,
            in_seq_no=in_seq_no,
            out_seq_no=out_seq_no,
            message=message,
        )


class SendMessageTypingAction(SecretTLObject):
    """``sendMessageTypingAction#16bf744e`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x16BF744E
    TL_NAME = "sendMessageTypingAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageCancelAction(SecretTLObject):
    """``sendMessageCancelAction#fd5ec8f5`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0xFD5EC8F5
    TL_NAME = "sendMessageCancelAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageRecordVideoAction(SecretTLObject):
    """``sendMessageRecordVideoAction#a187d66f`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0xA187D66F
    TL_NAME = "sendMessageRecordVideoAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageUploadVideoAction(SecretTLObject):
    """``sendMessageUploadVideoAction#92042ff7`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x92042FF7
    TL_NAME = "sendMessageUploadVideoAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageRecordAudioAction(SecretTLObject):
    """``sendMessageRecordAudioAction#d52f73f7`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0xD52F73F7
    TL_NAME = "sendMessageRecordAudioAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageUploadAudioAction(SecretTLObject):
    """``sendMessageUploadAudioAction#e6ac8a6f`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0xE6AC8A6F
    TL_NAME = "sendMessageUploadAudioAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageUploadPhotoAction(SecretTLObject):
    """``sendMessageUploadPhotoAction#990a3c1a`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x990A3C1A
    TL_NAME = "sendMessageUploadPhotoAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageUploadDocumentAction(SecretTLObject):
    """``sendMessageUploadDocumentAction#8faee98e`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x8FAEE98E
    TL_NAME = "sendMessageUploadDocumentAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageGeoLocationAction(SecretTLObject):
    """``sendMessageGeoLocationAction#176f8ba1`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x176F8BA1
    TL_NAME = "sendMessageGeoLocationAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageChooseContactAction(SecretTLObject):
    """``sendMessageChooseContactAction#628cbc6f`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x628CBC6F
    TL_NAME = "sendMessageChooseContactAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DecryptedMessageActionResend(SecretTLObject):
    """``decryptedMessageActionResend#511110b0`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x511110B0
    TL_NAME = "decryptedMessageActionResend"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, start_seq_no=None, end_seq_no=None):
        self.start_seq_no = start_seq_no
        self.end_seq_no = end_seq_no

    def _write_body(self, w):
        w.append(_pack_int(self.start_seq_no))
        w.append(_pack_int(self.end_seq_no))

    @classmethod
    def from_reader(cls, r):
        start_seq_no = r.read_int()
        end_seq_no = r.read_int()
        return cls(start_seq_no=start_seq_no, end_seq_no=end_seq_no)


class DecryptedMessageActionNotifyLayer(SecretTLObject):
    """``decryptedMessageActionNotifyLayer#f3048883`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xF3048883
    TL_NAME = "decryptedMessageActionNotifyLayer"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, layer=None):
        self.layer = layer

    def _write_body(self, w):
        w.append(_pack_int(self.layer))

    @classmethod
    def from_reader(cls, r):
        layer = r.read_int()
        return cls(layer=layer)


class DecryptedMessageActionTyping(SecretTLObject):
    """``decryptedMessageActionTyping#ccb27641`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xCCB27641
    TL_NAME = "decryptedMessageActionTyping"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, action=None):
        self.action = action

    def _write_body(self, w):
        w.append(bytes(self.action))

    @classmethod
    def from_reader(cls, r):
        action = read_object(r)
        return cls(action=action)


class DecryptedMessageActionRequestKey(SecretTLObject):
    """``decryptedMessageActionRequestKey#f3c9611b`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xF3C9611B
    TL_NAME = "decryptedMessageActionRequestKey"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, exchange_id=None, g_a=None):
        self.exchange_id = exchange_id
        self.g_a = g_a

    def _write_body(self, w):
        w.append(_pack_long(self.exchange_id))
        w.append(_serialize_bytes(self.g_a))

    @classmethod
    def from_reader(cls, r):
        exchange_id = r.read_long()
        g_a = r.tgread_bytes()
        return cls(exchange_id=exchange_id, g_a=g_a)


class DecryptedMessageActionAcceptKey(SecretTLObject):
    """``decryptedMessageActionAcceptKey#6fe1735b`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0x6FE1735B
    TL_NAME = "decryptedMessageActionAcceptKey"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, exchange_id=None, g_b=None, key_fingerprint=None):
        self.exchange_id = exchange_id
        self.g_b = g_b
        self.key_fingerprint = key_fingerprint

    def _write_body(self, w):
        w.append(_pack_long(self.exchange_id))
        w.append(_serialize_bytes(self.g_b))
        w.append(_pack_long(self.key_fingerprint))

    @classmethod
    def from_reader(cls, r):
        exchange_id = r.read_long()
        g_b = r.tgread_bytes()
        key_fingerprint = r.read_long()
        return cls(exchange_id=exchange_id, g_b=g_b, key_fingerprint=key_fingerprint)


class DecryptedMessageActionAbortKey(SecretTLObject):
    """``decryptedMessageActionAbortKey#dd05ec6b`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xDD05EC6B
    TL_NAME = "decryptedMessageActionAbortKey"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, exchange_id=None):
        self.exchange_id = exchange_id

    def _write_body(self, w):
        w.append(_pack_long(self.exchange_id))

    @classmethod
    def from_reader(cls, r):
        exchange_id = r.read_long()
        return cls(exchange_id=exchange_id)


class DecryptedMessageActionCommitKey(SecretTLObject):
    """``decryptedMessageActionCommitKey#ec2e0b9b`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xEC2E0B9B
    TL_NAME = "decryptedMessageActionCommitKey"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self, exchange_id=None, key_fingerprint=None):
        self.exchange_id = exchange_id
        self.key_fingerprint = key_fingerprint

    def _write_body(self, w):
        w.append(_pack_long(self.exchange_id))
        w.append(_pack_long(self.key_fingerprint))

    @classmethod
    def from_reader(cls, r):
        exchange_id = r.read_long()
        key_fingerprint = r.read_long()
        return cls(exchange_id=exchange_id, key_fingerprint=key_fingerprint)


class DecryptedMessageActionNoop(SecretTLObject):
    """``decryptedMessageActionNoop#a82fdd63`` -> ``DecryptedMessageAction``."""

    CONSTRUCTOR_ID = 0xA82FDD63
    TL_NAME = "decryptedMessageActionNoop"
    RESULT_TYPE = "DecryptedMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DocumentAttributeImageSize(SecretTLObject):
    """``documentAttributeImageSize#6c37c15c`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x6C37C15C
    TL_NAME = "documentAttributeImageSize"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, w=None, h=None):
        self.w = w
        self.h = h

    def _write_body(self, w):
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))

    @classmethod
    def from_reader(cls, r):
        w = r.read_int()
        h = r.read_int()
        return cls(w=w, h=h)


class DocumentAttributeAnimated(SecretTLObject):
    """``documentAttributeAnimated#11b58939`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x11B58939
    TL_NAME = "documentAttributeAnimated"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DocumentAttributeSticker_fb0a5727(SecretTLObject):
    """``documentAttributeSticker#fb0a5727`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0xFB0A5727
    TL_NAME = "documentAttributeSticker"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DocumentAttributeVideo_5910cccb(SecretTLObject):
    """``documentAttributeVideo#5910cccb`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x5910CCCB
    TL_NAME = "documentAttributeVideo"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, duration=None, w=None, h=None):
        self.duration = duration
        self.w = w
        self.h = h

    def _write_body(self, w):
        w.append(_pack_int(self.duration))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))

    @classmethod
    def from_reader(cls, r):
        duration = r.read_int()
        w = r.read_int()
        h = r.read_int()
        return cls(duration=duration, w=w, h=h)


class DocumentAttributeAudio_51448e5(SecretTLObject):
    """``documentAttributeAudio#51448e5`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x051448E5
    TL_NAME = "documentAttributeAudio"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, duration=None):
        self.duration = duration

    def _write_body(self, w):
        w.append(_pack_int(self.duration))

    @classmethod
    def from_reader(cls, r):
        duration = r.read_int()
        return cls(duration=duration)


class DocumentAttributeFilename(SecretTLObject):
    """``documentAttributeFilename#15590068`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x15590068
    TL_NAME = "documentAttributeFilename"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, file_name=None):
        self.file_name = file_name

    def _write_body(self, w):
        w.append(_serialize_bytes(self.file_name))

    @classmethod
    def from_reader(cls, r):
        file_name = r.tgread_string()
        return cls(file_name=file_name)


class PhotoSizeEmpty(SecretTLObject):
    """``photoSizeEmpty#e17e23c`` -> ``PhotoSize``."""

    CONSTRUCTOR_ID = 0x0E17E23C
    TL_NAME = "photoSizeEmpty"
    RESULT_TYPE = "PhotoSize"

    def __init__(self, type=None):
        self.type = type

    def _write_body(self, w):
        w.append(_serialize_bytes(self.type))

    @classmethod
    def from_reader(cls, r):
        type = r.tgread_string()
        return cls(type=type)


class PhotoSize(SecretTLObject):
    """``photoSize#77bfb61b`` -> ``PhotoSize``."""

    CONSTRUCTOR_ID = 0x77BFB61B
    TL_NAME = "photoSize"
    RESULT_TYPE = "PhotoSize"

    def __init__(self, type=None, location=None, w=None, h=None, size=None):
        self.type = type
        self.location = location
        self.w = w
        self.h = h
        self.size = size

    def _write_body(self, w):
        w.append(_serialize_bytes(self.type))
        w.append(bytes(self.location))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))

    @classmethod
    def from_reader(cls, r):
        type = r.tgread_string()
        location = read_object(r)
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        return cls(type=type, location=location, w=w, h=h, size=size)


class PhotoCachedSize(SecretTLObject):
    """``photoCachedSize#e9a734fa`` -> ``PhotoSize``."""

    CONSTRUCTOR_ID = 0xE9A734FA
    TL_NAME = "photoCachedSize"
    RESULT_TYPE = "PhotoSize"

    def __init__(self, type=None, location=None, w=None, h=None, bytes=None):
        self.type = type
        self.location = location
        self.w = w
        self.h = h
        self.bytes = bytes

    def _write_body(self, w):
        w.append(_serialize_bytes(self.type))
        w.append(bytes(self.location))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_serialize_bytes(self.bytes))

    @classmethod
    def from_reader(cls, r):
        type = r.tgread_string()
        location = read_object(r)
        w = r.read_int()
        h = r.read_int()
        bytes = r.tgread_bytes()
        return cls(type=type, location=location, w=w, h=h, bytes=bytes)


class FileLocationUnavailable(SecretTLObject):
    """``fileLocationUnavailable#7c596b46`` -> ``FileLocation``."""

    CONSTRUCTOR_ID = 0x7C596B46
    TL_NAME = "fileLocationUnavailable"
    RESULT_TYPE = "FileLocation"

    def __init__(self, volume_id=None, local_id=None, secret=None):
        self.volume_id = volume_id
        self.local_id = local_id
        self.secret = secret

    def _write_body(self, w):
        w.append(_pack_long(self.volume_id))
        w.append(_pack_int(self.local_id))
        w.append(_pack_long(self.secret))

    @classmethod
    def from_reader(cls, r):
        volume_id = r.read_long()
        local_id = r.read_int()
        secret = r.read_long()
        return cls(volume_id=volume_id, local_id=local_id, secret=secret)


class FileLocation(SecretTLObject):
    """``fileLocation#53d69076`` -> ``FileLocation``."""

    CONSTRUCTOR_ID = 0x53D69076
    TL_NAME = "fileLocation"
    RESULT_TYPE = "FileLocation"

    def __init__(self, dc_id=None, volume_id=None, local_id=None, secret=None):
        self.dc_id = dc_id
        self.volume_id = volume_id
        self.local_id = local_id
        self.secret = secret

    def _write_body(self, w):
        w.append(_pack_int(self.dc_id))
        w.append(_pack_long(self.volume_id))
        w.append(_pack_int(self.local_id))
        w.append(_pack_long(self.secret))

    @classmethod
    def from_reader(cls, r):
        dc_id = r.read_int()
        volume_id = r.read_long()
        local_id = r.read_int()
        secret = r.read_long()
        return cls(dc_id=dc_id, volume_id=volume_id, local_id=local_id, secret=secret)


class DecryptedMessageMediaExternalDocument(SecretTLObject):
    """``decryptedMessageMediaExternalDocument#fa95b0dd`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0xFA95B0DD
    TL_NAME = "decryptedMessageMediaExternalDocument"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        id=None,
        access_hash=None,
        date=None,
        mime_type=None,
        size=None,
        thumb=None,
        dc_id=None,
        attributes=None,
    ):
        self.id = id
        self.access_hash = access_hash
        self.date = date
        self.mime_type = mime_type
        self.size = size
        self.thumb = thumb
        self.dc_id = dc_id
        self.attributes = attributes

    def _write_body(self, w):
        w.append(_pack_long(self.id))
        w.append(_pack_long(self.access_hash))
        w.append(_pack_int(self.date))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.size))
        w.append(bytes(self.thumb))
        w.append(_pack_int(self.dc_id))
        w.append(_VECTOR)
        w.append(_pack_int(len(self.attributes)))
        for _item in self.attributes:
            w.append(bytes(_item))

    @classmethod
    def from_reader(cls, r):
        id = r.read_long()
        access_hash = r.read_long()
        date = r.read_int()
        mime_type = r.tgread_string()
        size = r.read_int()
        thumb = read_object(r)
        dc_id = r.read_int()
        attributes = _read_vector(r, lambda: read_object(r))
        return cls(
            id=id,
            access_hash=access_hash,
            date=date,
            mime_type=mime_type,
            size=size,
            thumb=thumb,
            dc_id=dc_id,
            attributes=attributes,
        )


class DecryptedMessage_36b091de(SecretTLObject):
    """``decryptedMessage#36b091de`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0x36B091DE
    TL_NAME = "decryptedMessage"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(
        self,
        random_id=None,
        ttl=None,
        message=None,
        media=None,
        entities=None,
        via_bot_name=None,
        reply_to_random_id=None,
    ):
        self.random_id = random_id
        self.ttl = ttl
        self.message = message
        self.media = media
        self.entities = entities
        self.via_bot_name = via_bot_name
        self.reply_to_random_id = reply_to_random_id

    def _write_body(self, w):
        flags = (
            (1 << 9 if self.media is not None else 0)
            | (1 << 7 if self.entities is not None else 0)
            | (1 << 11 if self.via_bot_name is not None else 0)
            | (1 << 3 if self.reply_to_random_id is not None else 0)
        )
        w.append(_pack_int(flags))
        w.append(_pack_long(self.random_id))
        w.append(_pack_int(self.ttl))
        w.append(_serialize_bytes(self.message))
        if self.media is not None:
            w.append(bytes(self.media))
        if self.entities is not None:
            w.append(_VECTOR)
            w.append(_pack_int(len(self.entities)))
            for _item in self.entities:
                w.append(bytes(_item))
        if self.via_bot_name is not None:
            w.append(_serialize_bytes(self.via_bot_name))
        if self.reply_to_random_id is not None:
            w.append(_pack_long(self.reply_to_random_id))

    @classmethod
    def from_reader(cls, r):
        flags = r.read_int()
        random_id = r.read_long()
        ttl = r.read_int()
        message = r.tgread_string()
        media = read_object(r) if flags & (1 << 9) else None
        entities = _read_vector(r, lambda: read_object(r)) if flags & (1 << 7) else None
        via_bot_name = r.tgread_string() if flags & (1 << 11) else None
        reply_to_random_id = r.read_long() if flags & (1 << 3) else None
        return cls(
            random_id=random_id,
            ttl=ttl,
            message=message,
            media=media,
            entities=entities,
            via_bot_name=via_bot_name,
            reply_to_random_id=reply_to_random_id,
        )


class DecryptedMessageMediaPhoto(SecretTLObject):
    """``decryptedMessageMediaPhoto#f1fa8d78`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0xF1FA8D78
    TL_NAME = "decryptedMessageMediaPhoto"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        w=None,
        h=None,
        size=None,
        key=None,
        iv=None,
        caption=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.w = w
        self.h = h
        self.size = size
        self.key = key
        self.iv = iv
        self.caption = caption

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))
        w.append(_serialize_bytes(self.caption))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        caption = r.tgread_string()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            w=w,
            h=h,
            size=size,
            key=key,
            iv=iv,
            caption=caption,
        )


class DecryptedMessageMediaVideo(SecretTLObject):
    """``decryptedMessageMediaVideo#970c8c0e`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x970C8C0E
    TL_NAME = "decryptedMessageMediaVideo"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        duration=None,
        mime_type=None,
        w=None,
        h=None,
        size=None,
        key=None,
        iv=None,
        caption=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.duration = duration
        self.mime_type = mime_type
        self.w = w
        self.h = h
        self.size = size
        self.key = key
        self.iv = iv
        self.caption = caption

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_pack_int(self.duration))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))
        w.append(_serialize_bytes(self.caption))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        duration = r.read_int()
        mime_type = r.tgread_string()
        w = r.read_int()
        h = r.read_int()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        caption = r.tgread_string()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            duration=duration,
            mime_type=mime_type,
            w=w,
            h=h,
            size=size,
            key=key,
            iv=iv,
            caption=caption,
        )


class DecryptedMessageMediaDocument_7afe8ae2(SecretTLObject):
    """``decryptedMessageMediaDocument#7afe8ae2`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x7AFE8AE2
    TL_NAME = "decryptedMessageMediaDocument"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        mime_type=None,
        size=None,
        key=None,
        iv=None,
        attributes=None,
        caption=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.mime_type = mime_type
        self.size = size
        self.key = key
        self.iv = iv
        self.attributes = attributes
        self.caption = caption

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_int(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))
        w.append(_VECTOR)
        w.append(_pack_int(len(self.attributes)))
        for _item in self.attributes:
            w.append(bytes(_item))
        w.append(_serialize_bytes(self.caption))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        mime_type = r.tgread_string()
        size = r.read_int()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        attributes = _read_vector(r, lambda: read_object(r))
        caption = r.tgread_string()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            mime_type=mime_type,
            size=size,
            key=key,
            iv=iv,
            attributes=attributes,
            caption=caption,
        )


class DocumentAttributeSticker(SecretTLObject):
    """``documentAttributeSticker#3a556302`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x3A556302
    TL_NAME = "documentAttributeSticker"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, alt=None, stickerset=None):
        self.alt = alt
        self.stickerset = stickerset

    def _write_body(self, w):
        w.append(_serialize_bytes(self.alt))
        w.append(bytes(self.stickerset))

    @classmethod
    def from_reader(cls, r):
        alt = r.tgread_string()
        stickerset = read_object(r)
        return cls(alt=alt, stickerset=stickerset)


class DocumentAttributeAudio_ded218e0(SecretTLObject):
    """``documentAttributeAudio#ded218e0`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0xDED218E0
    TL_NAME = "documentAttributeAudio"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, duration=None, title=None, performer=None):
        self.duration = duration
        self.title = title
        self.performer = performer

    def _write_body(self, w):
        w.append(_pack_int(self.duration))
        w.append(_serialize_bytes(self.title))
        w.append(_serialize_bytes(self.performer))

    @classmethod
    def from_reader(cls, r):
        duration = r.read_int()
        title = r.tgread_string()
        performer = r.tgread_string()
        return cls(duration=duration, title=title, performer=performer)


class MessageEntityUnknown(SecretTLObject):
    """``messageEntityUnknown#bb92ba95`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0xBB92BA95
    TL_NAME = "messageEntityUnknown"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityMention(SecretTLObject):
    """``messageEntityMention#fa04579d`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0xFA04579D
    TL_NAME = "messageEntityMention"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityHashtag(SecretTLObject):
    """``messageEntityHashtag#6f635b0d`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x6F635B0D
    TL_NAME = "messageEntityHashtag"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityBotCommand(SecretTLObject):
    """``messageEntityBotCommand#6cef8ac7`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x6CEF8AC7
    TL_NAME = "messageEntityBotCommand"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityUrl(SecretTLObject):
    """``messageEntityUrl#6ed02538`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x6ED02538
    TL_NAME = "messageEntityUrl"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityEmail(SecretTLObject):
    """``messageEntityEmail#64e475c2`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x64E475C2
    TL_NAME = "messageEntityEmail"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityBold(SecretTLObject):
    """``messageEntityBold#bd610bc9`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0xBD610BC9
    TL_NAME = "messageEntityBold"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityItalic(SecretTLObject):
    """``messageEntityItalic#826f8b60`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x826F8B60
    TL_NAME = "messageEntityItalic"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityCode(SecretTLObject):
    """``messageEntityCode#28a20571`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x28A20571
    TL_NAME = "messageEntityCode"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityPre(SecretTLObject):
    """``messageEntityPre#73924be0`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x73924BE0
    TL_NAME = "messageEntityPre"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None, language=None):
        self.offset = offset
        self.length = length
        self.language = language

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))
        w.append(_serialize_bytes(self.language))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        language = r.tgread_string()
        return cls(offset=offset, length=length, language=language)


class MessageEntityTextUrl(SecretTLObject):
    """``messageEntityTextUrl#76a6d327`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x76A6D327
    TL_NAME = "messageEntityTextUrl"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None, url=None):
        self.offset = offset
        self.length = length
        self.url = url

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))
        w.append(_serialize_bytes(self.url))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        url = r.tgread_string()
        return cls(offset=offset, length=length, url=url)


class InputStickerSetShortName(SecretTLObject):
    """``inputStickerSetShortName#861cc8a0`` -> ``InputStickerSet``."""

    CONSTRUCTOR_ID = 0x861CC8A0
    TL_NAME = "inputStickerSetShortName"
    RESULT_TYPE = "InputStickerSet"

    def __init__(self, short_name=None):
        self.short_name = short_name

    def _write_body(self, w):
        w.append(_serialize_bytes(self.short_name))

    @classmethod
    def from_reader(cls, r):
        short_name = r.tgread_string()
        return cls(short_name=short_name)


class InputStickerSetEmpty(SecretTLObject):
    """``inputStickerSetEmpty#ffb62b95`` -> ``InputStickerSet``."""

    CONSTRUCTOR_ID = 0xFFB62B95
    TL_NAME = "inputStickerSetEmpty"
    RESULT_TYPE = "InputStickerSet"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DecryptedMessageMediaVenue(SecretTLObject):
    """``decryptedMessageMediaVenue#8a0df56f`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x8A0DF56F
    TL_NAME = "decryptedMessageMediaVenue"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self, lat=None, long=None, title=None, address=None, provider=None, venue_id=None
    ):
        self.lat = lat
        self.long = long
        self.title = title
        self.address = address
        self.provider = provider
        self.venue_id = venue_id

    def _write_body(self, w):
        w.append(_pack_double(self.lat))
        w.append(_pack_double(self.long))
        w.append(_serialize_bytes(self.title))
        w.append(_serialize_bytes(self.address))
        w.append(_serialize_bytes(self.provider))
        w.append(_serialize_bytes(self.venue_id))

    @classmethod
    def from_reader(cls, r):
        lat = r.read_double()
        long = r.read_double()
        title = r.tgread_string()
        address = r.tgread_string()
        provider = r.tgread_string()
        venue_id = r.tgread_string()
        return cls(
            lat=lat, long=long, title=title, address=address, provider=provider, venue_id=venue_id
        )


class DecryptedMessageMediaWebPage(SecretTLObject):
    """``decryptedMessageMediaWebPage#e50511d8`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0xE50511D8
    TL_NAME = "decryptedMessageMediaWebPage"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(self, url=None):
        self.url = url

    def _write_body(self, w):
        w.append(_serialize_bytes(self.url))

    @classmethod
    def from_reader(cls, r):
        url = r.tgread_string()
        return cls(url=url)


class DocumentAttributeAudio(SecretTLObject):
    """``documentAttributeAudio#9852f9c6`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x9852F9C6
    TL_NAME = "documentAttributeAudio"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, voice=None, duration=None, title=None, performer=None, waveform=None):
        self.voice = voice
        self.duration = duration
        self.title = title
        self.performer = performer
        self.waveform = waveform

    def _write_body(self, w):
        flags = (
            (1 << 10 if self.voice else 0)
            | (1 << 0 if self.title is not None else 0)
            | (1 << 1 if self.performer is not None else 0)
            | (1 << 2 if self.waveform is not None else 0)
        )
        w.append(_pack_int(flags))
        w.append(_pack_int(self.duration))
        if self.title is not None:
            w.append(_serialize_bytes(self.title))
        if self.performer is not None:
            w.append(_serialize_bytes(self.performer))
        if self.waveform is not None:
            w.append(_serialize_bytes(self.waveform))

    @classmethod
    def from_reader(cls, r):
        flags = r.read_int()
        voice = bool(flags & (1 << 10))
        duration = r.read_int()
        title = r.tgread_string() if flags & (1 << 0) else None
        performer = r.tgread_string() if flags & (1 << 1) else None
        waveform = r.tgread_bytes() if flags & (1 << 2) else None
        return cls(
            voice=voice, duration=duration, title=title, performer=performer, waveform=waveform
        )


class DocumentAttributeVideo(SecretTLObject):
    """``documentAttributeVideo#ef02ce6`` -> ``DocumentAttribute``."""

    CONSTRUCTOR_ID = 0x0EF02CE6
    TL_NAME = "documentAttributeVideo"
    RESULT_TYPE = "DocumentAttribute"

    def __init__(self, round_message=None, duration=None, w=None, h=None):
        self.round_message = round_message
        self.duration = duration
        self.w = w
        self.h = h

    def _write_body(self, w):
        flags = 1 << 0 if self.round_message else 0
        w.append(_pack_int(flags))
        w.append(_pack_int(self.duration))
        w.append(_pack_int(self.w))
        w.append(_pack_int(self.h))

    @classmethod
    def from_reader(cls, r):
        flags = r.read_int()
        round_message = bool(flags & (1 << 0))
        duration = r.read_int()
        w = r.read_int()
        h = r.read_int()
        return cls(round_message=round_message, duration=duration, w=w, h=h)


class SendMessageRecordRoundAction(SecretTLObject):
    """``sendMessageRecordRoundAction#88f27fbc`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0x88F27FBC
    TL_NAME = "sendMessageRecordRoundAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class SendMessageUploadRoundAction(SecretTLObject):
    """``sendMessageUploadRoundAction#bb718624`` -> ``SendMessageAction``."""

    CONSTRUCTOR_ID = 0xBB718624
    TL_NAME = "sendMessageUploadRoundAction"
    RESULT_TYPE = "SendMessageAction"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class DecryptedMessage(SecretTLObject):
    """``decryptedMessage#91cc4674`` -> ``DecryptedMessage``."""

    CONSTRUCTOR_ID = 0x91CC4674
    TL_NAME = "decryptedMessage"
    RESULT_TYPE = "DecryptedMessage"

    def __init__(
        self,
        no_webpage=None,
        silent=None,
        random_id=None,
        ttl=None,
        message=None,
        media=None,
        entities=None,
        via_bot_name=None,
        reply_to_random_id=None,
        grouped_id=None,
    ):
        self.no_webpage = no_webpage
        self.silent = silent
        self.random_id = random_id
        self.ttl = ttl
        self.message = message
        self.media = media
        self.entities = entities
        self.via_bot_name = via_bot_name
        self.reply_to_random_id = reply_to_random_id
        self.grouped_id = grouped_id

    def _write_body(self, w):
        flags = (
            (1 << 1 if self.no_webpage else 0)
            | (1 << 5 if self.silent else 0)
            | (1 << 9 if self.media is not None else 0)
            | (1 << 7 if self.entities is not None else 0)
            | (1 << 11 if self.via_bot_name is not None else 0)
            | (1 << 3 if self.reply_to_random_id is not None else 0)
            | (1 << 17 if self.grouped_id is not None else 0)
        )
        w.append(_pack_int(flags))
        w.append(_pack_long(self.random_id))
        w.append(_pack_int(self.ttl))
        w.append(_serialize_bytes(self.message))
        if self.media is not None:
            w.append(bytes(self.media))
        if self.entities is not None:
            w.append(_VECTOR)
            w.append(_pack_int(len(self.entities)))
            for _item in self.entities:
                w.append(bytes(_item))
        if self.via_bot_name is not None:
            w.append(_serialize_bytes(self.via_bot_name))
        if self.reply_to_random_id is not None:
            w.append(_pack_long(self.reply_to_random_id))
        if self.grouped_id is not None:
            w.append(_pack_long(self.grouped_id))

    @classmethod
    def from_reader(cls, r):
        flags = r.read_int()
        no_webpage = bool(flags & (1 << 1))
        silent = bool(flags & (1 << 5))
        random_id = r.read_long()
        ttl = r.read_int()
        message = r.tgread_string()
        media = read_object(r) if flags & (1 << 9) else None
        entities = _read_vector(r, lambda: read_object(r)) if flags & (1 << 7) else None
        via_bot_name = r.tgread_string() if flags & (1 << 11) else None
        reply_to_random_id = r.read_long() if flags & (1 << 3) else None
        grouped_id = r.read_long() if flags & (1 << 17) else None
        return cls(
            no_webpage=no_webpage,
            silent=silent,
            random_id=random_id,
            ttl=ttl,
            message=message,
            media=media,
            entities=entities,
            via_bot_name=via_bot_name,
            reply_to_random_id=reply_to_random_id,
            grouped_id=grouped_id,
        )


class MessageEntityUnderline(SecretTLObject):
    """``messageEntityUnderline#9c4e7e8b`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x9C4E7E8B
    TL_NAME = "messageEntityUnderline"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityStrike(SecretTLObject):
    """``messageEntityStrike#bf0693d4`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0xBF0693D4
    TL_NAME = "messageEntityStrike"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityBlockquote(SecretTLObject):
    """``messageEntityBlockquote#20df5d0`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x020DF5D0
    TL_NAME = "messageEntityBlockquote"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class DecryptedMessageMediaDocument(SecretTLObject):
    """``decryptedMessageMediaDocument#6abd9782`` -> ``DecryptedMessageMedia``."""

    CONSTRUCTOR_ID = 0x6ABD9782
    TL_NAME = "decryptedMessageMediaDocument"
    RESULT_TYPE = "DecryptedMessageMedia"

    def __init__(
        self,
        thumb=None,
        thumb_w=None,
        thumb_h=None,
        mime_type=None,
        size=None,
        key=None,
        iv=None,
        attributes=None,
        caption=None,
    ):
        self.thumb = thumb
        self.thumb_w = thumb_w
        self.thumb_h = thumb_h
        self.mime_type = mime_type
        self.size = size
        self.key = key
        self.iv = iv
        self.attributes = attributes
        self.caption = caption

    def _write_body(self, w):
        w.append(_serialize_bytes(self.thumb))
        w.append(_pack_int(self.thumb_w))
        w.append(_pack_int(self.thumb_h))
        w.append(_serialize_bytes(self.mime_type))
        w.append(_pack_long(self.size))
        w.append(_serialize_bytes(self.key))
        w.append(_serialize_bytes(self.iv))
        w.append(_VECTOR)
        w.append(_pack_int(len(self.attributes)))
        for _item in self.attributes:
            w.append(bytes(_item))
        w.append(_serialize_bytes(self.caption))

    @classmethod
    def from_reader(cls, r):
        thumb = r.tgread_bytes()
        thumb_w = r.read_int()
        thumb_h = r.read_int()
        mime_type = r.tgread_string()
        size = r.read_long()
        key = r.tgread_bytes()
        iv = r.tgread_bytes()
        attributes = _read_vector(r, lambda: read_object(r))
        caption = r.tgread_string()
        return cls(
            thumb=thumb,
            thumb_w=thumb_w,
            thumb_h=thumb_h,
            mime_type=mime_type,
            size=size,
            key=key,
            iv=iv,
            attributes=attributes,
            caption=caption,
        )


class MessageEntitySpoiler(SecretTLObject):
    """``messageEntitySpoiler#32ca960f`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0x32CA960F
    TL_NAME = "messageEntitySpoiler"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None):
        self.offset = offset
        self.length = length

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        return cls(offset=offset, length=length)


class MessageEntityCustomEmoji(SecretTLObject):
    """``messageEntityCustomEmoji#c8cf05f8`` -> ``MessageEntity``."""

    CONSTRUCTOR_ID = 0xC8CF05F8
    TL_NAME = "messageEntityCustomEmoji"
    RESULT_TYPE = "MessageEntity"

    def __init__(self, offset=None, length=None, document_id=None):
        self.offset = offset
        self.length = length
        self.document_id = document_id

    def _write_body(self, w):
        w.append(_pack_int(self.offset))
        w.append(_pack_int(self.length))
        w.append(_pack_long(self.document_id))

    @classmethod
    def from_reader(cls, r):
        offset = r.read_int()
        length = r.read_int()
        document_id = r.read_long()
        return cls(offset=offset, length=length, document_id=document_id)


class JsonObjectValue(SecretTLObject):
    """``jsonObjectValue#c0de1bd9`` -> ``JSONObjectValue``."""

    CONSTRUCTOR_ID = 0xC0DE1BD9
    TL_NAME = "jsonObjectValue"
    RESULT_TYPE = "JSONObjectValue"

    def __init__(self, key=None, value=None):
        self.key = key
        self.value = value

    def _write_body(self, w):
        w.append(_serialize_bytes(self.key))
        w.append(bytes(self.value))

    @classmethod
    def from_reader(cls, r):
        key = r.tgread_string()
        value = read_object(r)
        return cls(key=key, value=value)


class JsonNull(SecretTLObject):
    """``jsonNull#3f6d7b68`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0x3F6D7B68
    TL_NAME = "jsonNull"
    RESULT_TYPE = "JSONValue"

    def __init__(self):
        pass

    def _write_body(self, w):
        return

    @classmethod
    def from_reader(cls, r):
        return cls()


class JsonBool(SecretTLObject):
    """``jsonBool#c7345e6a`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0xC7345E6A
    TL_NAME = "jsonBool"
    RESULT_TYPE = "JSONValue"

    def __init__(self, value=None):
        self.value = value

    def _write_body(self, w):
        w.append(_BOOL_TRUE if self.value else _BOOL_FALSE)

    @classmethod
    def from_reader(cls, r):
        value = r.tgread_bool()
        return cls(value=value)


class JsonNumber(SecretTLObject):
    """``jsonNumber#2be0dfa4`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0x2BE0DFA4
    TL_NAME = "jsonNumber"
    RESULT_TYPE = "JSONValue"

    def __init__(self, value=None):
        self.value = value

    def _write_body(self, w):
        w.append(_pack_double(self.value))

    @classmethod
    def from_reader(cls, r):
        value = r.read_double()
        return cls(value=value)


class JsonString(SecretTLObject):
    """``jsonString#b71e767a`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0xB71E767A
    TL_NAME = "jsonString"
    RESULT_TYPE = "JSONValue"

    def __init__(self, value=None):
        self.value = value

    def _write_body(self, w):
        w.append(_serialize_bytes(self.value))

    @classmethod
    def from_reader(cls, r):
        value = r.tgread_string()
        return cls(value=value)


class JsonArray(SecretTLObject):
    """``jsonArray#f7444763`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0xF7444763
    TL_NAME = "jsonArray"
    RESULT_TYPE = "JSONValue"

    def __init__(self, value=None):
        self.value = value

    def _write_body(self, w):
        w.append(_VECTOR)
        w.append(_pack_int(len(self.value)))
        for _item in self.value:
            w.append(bytes(_item))

    @classmethod
    def from_reader(cls, r):
        value = _read_vector(r, lambda: read_object(r))
        return cls(value=value)


class JsonObject(SecretTLObject):
    """``jsonObject#99c1d49d`` -> ``JSONValue``."""

    CONSTRUCTOR_ID = 0x99C1D49D
    TL_NAME = "jsonObject"
    RESULT_TYPE = "JSONValue"

    def __init__(self, value=None):
        self.value = value

    def _write_body(self, w):
        w.append(_VECTOR)
        w.append(_pack_int(len(self.value)))
        for _item in self.value:
            w.append(bytes(_item))

    @classmethod
    def from_reader(cls, r):
        value = _read_vector(r, lambda: read_object(r))
        return cls(value=value)


class TextWithEntities(SecretTLObject):
    """``textWithEntities#751f3146`` -> ``TextWithEntities``."""

    CONSTRUCTOR_ID = 0x751F3146
    TL_NAME = "textWithEntities"
    RESULT_TYPE = "TextWithEntities"

    def __init__(self, text=None, entities=None):
        self.text = text
        self.entities = entities

    def _write_body(self, w):
        w.append(_serialize_bytes(self.text))
        w.append(_VECTOR)
        w.append(_pack_int(len(self.entities)))
        for _item in self.entities:
            w.append(bytes(_item))

    @classmethod
    def from_reader(cls, r):
        text = r.tgread_string()
        entities = _read_vector(r, lambda: read_object(r))
        return cls(text=text, entities=entities)


class GroupCallMessage(SecretTLObject):
    """``groupCallMessage#907ce88e`` -> ``GroupCallMessage``."""

    CONSTRUCTOR_ID = 0x907CE88E
    TL_NAME = "groupCallMessage"
    RESULT_TYPE = "GroupCallMessage"

    def __init__(self, random_id=None, message=None):
        self.random_id = random_id
        self.message = message

    def _write_body(self, w):
        w.append(_pack_long(self.random_id))
        w.append(bytes(self.message))

    @classmethod
    def from_reader(cls, r):
        random_id = r.read_long()
        message = read_object(r)
        return cls(random_id=random_id, message=message)


REGISTRY = {
    0x1F814F1F: DecryptedMessage_1f814f1f,
    0xAA48327D: DecryptedMessageService8,
    0x089F5C4A: DecryptedMessageMediaEmpty,
    0x32798A8C: DecryptedMessageMediaPhoto_32798a8c,
    0x4CEE6EF3: DecryptedMessageMediaVideo_4cee6ef3,
    0x35480A59: DecryptedMessageMediaGeoPoint,
    0x588A0A97: DecryptedMessageMediaContact,
    0xA1733AEC: DecryptedMessageActionSetMessageTTL,
    0xB095434B: DecryptedMessageMediaDocument_b095434b,
    0x6080758F: DecryptedMessageMediaAudio_6080758f,
    0x0C4F40BE: DecryptedMessageActionReadMessages,
    0x65614304: DecryptedMessageActionDeleteMessages,
    0x8AC1F475: DecryptedMessageActionScreenshotMessages,
    0x6719E45C: DecryptedMessageActionFlushHistory,
    0x204D3878: DecryptedMessage_204d3878,
    0x73164160: DecryptedMessageService,
    0x524A415D: DecryptedMessageMediaVideo_524a415d,
    0x57E0A9CB: DecryptedMessageMediaAudio,
    0x1BE31789: DecryptedMessageLayer,
    0x16BF744E: SendMessageTypingAction,
    0xFD5EC8F5: SendMessageCancelAction,
    0xA187D66F: SendMessageRecordVideoAction,
    0x92042FF7: SendMessageUploadVideoAction,
    0xD52F73F7: SendMessageRecordAudioAction,
    0xE6AC8A6F: SendMessageUploadAudioAction,
    0x990A3C1A: SendMessageUploadPhotoAction,
    0x8FAEE98E: SendMessageUploadDocumentAction,
    0x176F8BA1: SendMessageGeoLocationAction,
    0x628CBC6F: SendMessageChooseContactAction,
    0x511110B0: DecryptedMessageActionResend,
    0xF3048883: DecryptedMessageActionNotifyLayer,
    0xCCB27641: DecryptedMessageActionTyping,
    0xF3C9611B: DecryptedMessageActionRequestKey,
    0x6FE1735B: DecryptedMessageActionAcceptKey,
    0xDD05EC6B: DecryptedMessageActionAbortKey,
    0xEC2E0B9B: DecryptedMessageActionCommitKey,
    0xA82FDD63: DecryptedMessageActionNoop,
    0x6C37C15C: DocumentAttributeImageSize,
    0x11B58939: DocumentAttributeAnimated,
    0xFB0A5727: DocumentAttributeSticker_fb0a5727,
    0x5910CCCB: DocumentAttributeVideo_5910cccb,
    0x051448E5: DocumentAttributeAudio_51448e5,
    0x15590068: DocumentAttributeFilename,
    0x0E17E23C: PhotoSizeEmpty,
    0x77BFB61B: PhotoSize,
    0xE9A734FA: PhotoCachedSize,
    0x7C596B46: FileLocationUnavailable,
    0x53D69076: FileLocation,
    0xFA95B0DD: DecryptedMessageMediaExternalDocument,
    0x36B091DE: DecryptedMessage_36b091de,
    0xF1FA8D78: DecryptedMessageMediaPhoto,
    0x970C8C0E: DecryptedMessageMediaVideo,
    0x7AFE8AE2: DecryptedMessageMediaDocument_7afe8ae2,
    0x3A556302: DocumentAttributeSticker,
    0xDED218E0: DocumentAttributeAudio_ded218e0,
    0xBB92BA95: MessageEntityUnknown,
    0xFA04579D: MessageEntityMention,
    0x6F635B0D: MessageEntityHashtag,
    0x6CEF8AC7: MessageEntityBotCommand,
    0x6ED02538: MessageEntityUrl,
    0x64E475C2: MessageEntityEmail,
    0xBD610BC9: MessageEntityBold,
    0x826F8B60: MessageEntityItalic,
    0x28A20571: MessageEntityCode,
    0x73924BE0: MessageEntityPre,
    0x76A6D327: MessageEntityTextUrl,
    0x861CC8A0: InputStickerSetShortName,
    0xFFB62B95: InputStickerSetEmpty,
    0x8A0DF56F: DecryptedMessageMediaVenue,
    0xE50511D8: DecryptedMessageMediaWebPage,
    0x9852F9C6: DocumentAttributeAudio,
    0x0EF02CE6: DocumentAttributeVideo,
    0x88F27FBC: SendMessageRecordRoundAction,
    0xBB718624: SendMessageUploadRoundAction,
    0x91CC4674: DecryptedMessage,
    0x9C4E7E8B: MessageEntityUnderline,
    0xBF0693D4: MessageEntityStrike,
    0x020DF5D0: MessageEntityBlockquote,
    0x6ABD9782: DecryptedMessageMediaDocument,
    0x32CA960F: MessageEntitySpoiler,
    0xC8CF05F8: MessageEntityCustomEmoji,
    0xC0DE1BD9: JsonObjectValue,
    0x3F6D7B68: JsonNull,
    0xC7345E6A: JsonBool,
    0x2BE0DFA4: JsonNumber,
    0xB71E767A: JsonString,
    0xF7444763: JsonArray,
    0x99C1D49D: JsonObject,
    0x751F3146: TextWithEntities,
    0x907CE88E: GroupCallMessage,
}

__all__ = [
    "DecryptedMessage_1f814f1f",
    "DecryptedMessageService8",
    "DecryptedMessageMediaEmpty",
    "DecryptedMessageMediaPhoto_32798a8c",
    "DecryptedMessageMediaVideo_4cee6ef3",
    "DecryptedMessageMediaGeoPoint",
    "DecryptedMessageMediaContact",
    "DecryptedMessageActionSetMessageTTL",
    "DecryptedMessageMediaDocument_b095434b",
    "DecryptedMessageMediaAudio_6080758f",
    "DecryptedMessageActionReadMessages",
    "DecryptedMessageActionDeleteMessages",
    "DecryptedMessageActionScreenshotMessages",
    "DecryptedMessageActionFlushHistory",
    "DecryptedMessage_204d3878",
    "DecryptedMessageService",
    "DecryptedMessageMediaVideo_524a415d",
    "DecryptedMessageMediaAudio",
    "DecryptedMessageLayer",
    "SendMessageTypingAction",
    "SendMessageCancelAction",
    "SendMessageRecordVideoAction",
    "SendMessageUploadVideoAction",
    "SendMessageRecordAudioAction",
    "SendMessageUploadAudioAction",
    "SendMessageUploadPhotoAction",
    "SendMessageUploadDocumentAction",
    "SendMessageGeoLocationAction",
    "SendMessageChooseContactAction",
    "DecryptedMessageActionResend",
    "DecryptedMessageActionNotifyLayer",
    "DecryptedMessageActionTyping",
    "DecryptedMessageActionRequestKey",
    "DecryptedMessageActionAcceptKey",
    "DecryptedMessageActionAbortKey",
    "DecryptedMessageActionCommitKey",
    "DecryptedMessageActionNoop",
    "DocumentAttributeImageSize",
    "DocumentAttributeAnimated",
    "DocumentAttributeSticker_fb0a5727",
    "DocumentAttributeVideo_5910cccb",
    "DocumentAttributeAudio_51448e5",
    "DocumentAttributeFilename",
    "PhotoSizeEmpty",
    "PhotoSize",
    "PhotoCachedSize",
    "FileLocationUnavailable",
    "FileLocation",
    "DecryptedMessageMediaExternalDocument",
    "DecryptedMessage_36b091de",
    "DecryptedMessageMediaPhoto",
    "DecryptedMessageMediaVideo",
    "DecryptedMessageMediaDocument_7afe8ae2",
    "DocumentAttributeSticker",
    "DocumentAttributeAudio_ded218e0",
    "MessageEntityUnknown",
    "MessageEntityMention",
    "MessageEntityHashtag",
    "MessageEntityBotCommand",
    "MessageEntityUrl",
    "MessageEntityEmail",
    "MessageEntityBold",
    "MessageEntityItalic",
    "MessageEntityCode",
    "MessageEntityPre",
    "MessageEntityTextUrl",
    "InputStickerSetShortName",
    "InputStickerSetEmpty",
    "DecryptedMessageMediaVenue",
    "DecryptedMessageMediaWebPage",
    "DocumentAttributeAudio",
    "DocumentAttributeVideo",
    "SendMessageRecordRoundAction",
    "SendMessageUploadRoundAction",
    "DecryptedMessage",
    "MessageEntityUnderline",
    "MessageEntityStrike",
    "MessageEntityBlockquote",
    "DecryptedMessageMediaDocument",
    "MessageEntitySpoiler",
    "MessageEntityCustomEmoji",
    "JsonObjectValue",
    "JsonNull",
    "JsonBool",
    "JsonNumber",
    "JsonString",
    "JsonArray",
    "JsonObject",
    "TextWithEntities",
    "GroupCallMessage",
    "REGISTRY",
    "SecretTLObject",
    "read_object",
    "UnknownConstructor",
]
