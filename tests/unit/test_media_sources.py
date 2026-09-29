"""A file to send may be a path, bytes, or a seekable stream (spec 005, US1).

No temporary file is written: the encrypting reader reads from whatever it is given, and a
stream's size comes from seeking, because the upload must declare it before the first byte.
"""

import io
import os

import pytest

from telethon_secret_chat import files

from .fake_client import establish


@pytest.fixture
async def ready(pair):
    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    return wire, a, b, chat_a, got


async def _plaintext(b, message, tmp_path, name):
    return (await b.save_file(message, tmp_path / name)).read_bytes()


async def test_bytes_and_a_stream_arrive_like_the_same_file_from_a_path(ready, tmp_path):
    wire, a, b, chat_a, got = ready
    content = os.urandom(3000)
    path = tmp_path / "clip.mp4"
    path.write_bytes(content)
    await a.send_file(chat_a.id, path, duration=3)
    await a.send_file(chat_a.id, content, file_name="clip.mp4", duration=3)
    await a.send_file(chat_a.id, io.BytesIO(content), file_name="clip.mp4", duration=3)
    assert len(got) == 3
    shapes = {(m.media.mime_type, m.media.size, repr(m.media.attributes)) for m in got}
    assert len(shapes) == 1
    for n, message in enumerate(got):
        assert await _plaintext(b, message, tmp_path, f"out{n}") == content


async def test_a_stream_sends_what_remains_from_its_position(ready, tmp_path):
    _, a, b, chat_a, got = ready
    stream = io.BytesIO(b"HEADER" + b"body" * 100)
    stream.seek(6)
    await a.send_file(chat_a.id, stream, file_name="notes.txt")
    assert await _plaintext(b, got[-1], tmp_path, "rest") == b"body" * 100


class _Unseekable(io.RawIOBase):
    def readable(self):
        return True

    def seekable(self):
        return False

    def readinto(self, buffer):
        return 0


@pytest.mark.parametrize(
    "source, kwargs",
    [
        (b"abc", {}),  # bytes without a name
        (io.BytesIO(b"abc"), {}),  # a stream without a name
        (_Unseekable(), {"file_name": "x.bin"}),
        (12345, {"file_name": "x.bin"}),  # not a file at all
    ],
)
async def test_an_unusable_source_is_refused_before_the_upload(ready, source, kwargs):
    wire, a, _, chat_a, _ = ready
    with pytest.raises(ValueError):
        await a.send_file(chat_a.id, source, **kwargs)
    assert wire.a.uploaded == []


def test_the_encrypting_reader_needs_no_temporary_file():
    """The reader encrypts from any readable object; bytes need no disk at all."""
    key, iv = files.new_file_key()
    blob = files.EncryptingReader(io.BytesIO(b"x" * 100), key, iv, 100).read()
    assert files.decrypt_file(blob, key, iv, 100) == b"x" * 100
