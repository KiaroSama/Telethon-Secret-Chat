"""Files, the generated parser and repr boundaries under hostile input."""

import hashlib
import struct
import subprocess
import sys

import pytest
from telethon.extensions import BinaryReader
from telethon.tl import types

from telethon_secret_chat import crypto, files, framing, rekey
from telethon_secret_chat.errors import MessageRejected
from telethon_secret_chat.events import MessageReceived, ServiceActionReceived
from telethon_secret_chat.schema import secret_tl as tl
from telethon_secret_chat.storage import FileStorage

from .fake_client import FakeClient
from .helpers import KEY, OTHER, ready_manager


async def test_file_storage_close_removes_live_key_material(tmp_path):
    path = tmp_path / "private.json"
    manager, chat = ready_manager(store=FileStorage(path))
    await manager.send_message(chat.id, "delete stored plaintext")
    await manager.close(chat.id)
    assert KEY.hex() not in path.read_text()
    assert "delete stored plaintext".encode().hex() not in path.read_text()
    assert FileStorage(path).load(chat.id)["key"] is None


@pytest.mark.parametrize(
    "layer,constructor",
    [(73, tl.DecryptedMessageMediaDocument_7afe8ae2), (143, tl.DecryptedMessageMediaDocument)],
)
async def test_media_constructor_matches_negotiated_layer(tmp_path, layer, constructor):
    manager, chat = ready_manager()
    chat.layer = layer
    path = tmp_path / "one.bin"
    path.write_bytes(b"sample")
    await manager.send_file(chat.id, path)
    wrapper = framing.unwrap(crypto.decrypt_frame(KEY, manager._client.sent[-1].data, chat.out_x))
    assert type(wrapper.message.media) is constructor


async def test_big_file_upload_uses_big_encrypted_handle(tmp_path):
    class Big(FakeClient):
        async def upload_file(self, file, **kwargs):
            return types.InputFileBig(id=5, parts=22, name="big.bin")

    manager, chat = ready_manager(Big())
    path = tmp_path / "small-fixture.bin"
    path.write_bytes(b"a small stand-in for the uploader's BIG result")
    await manager.send_file(chat.id, path)
    assert isinstance(manager._client.sent[-1].file, types.InputEncryptedFileBigUploaded)


async def test_receive_uses_file_dc_and_rejects_bad_fingerprint_before_download(tmp_path):
    class Recording(FakeClient):
        downloads = []

        async def download_file(self, location, out, **kwargs):
            self.downloads.append(kwargs)
            return await super().download_file(location, out, **kwargs)

    client = Recording()
    manager, chat = ready_manager(client)
    key, iv = files.new_file_key()
    client.stored[5] = files.encrypt_file(b"sample", key, iv)
    attached = types.EncryptedFile(
        id=5, access_hash=6, size=16, dc_id=4, key_fingerprint=files.file_fingerprint(key, iv)
    )
    media = tl.DecryptedMessageMediaDocument(key=key, iv=iv, size=6)
    event = MessageReceived(chat.id, 1, 1, "", media=media, file=attached)
    assert (await manager.save_file(event, tmp_path / "ok")).read_bytes() == b"sample"
    assert client.downloads == [{"dc_id": 4}]
    attached.key_fingerprint ^= 1
    with pytest.raises(MessageRejected):
        await manager.save_file(event, tmp_path / "bad")
    assert len(client.downloads) == 1


@pytest.mark.parametrize("size", [0, 1, 15, 16, 17, 255, 256, 257])
def test_file_length_boundaries_are_exact(size):
    key, iv = files.new_file_key()
    data = b"x" * size
    assert files.decrypt_file(files.encrypt_file(data, key, iv), key, iv, size) == data


def test_malformed_ciphertext_is_rejected_without_native_process_abort():
    result = subprocess.run(
        [
            sys.executable,
            "-c",
            "from telethon_secret_chat.files import decrypt_file\n"
            "try: decrypt_file(b'x'*17, b'k'*32, b'i'*32, 1)\n"
            "except ValueError: pass\n"
            "else: raise AssertionError('malformed ciphertext accepted')\n",
        ],
        capture_output=True,
        timeout=15,
    )
    assert result.returncode == 0, "invalid input reached the native cipher or was accepted"


@pytest.mark.parametrize("count", [-1, 2**31 - 1])
def test_generated_parser_rejects_impossible_vector_counts(count):
    data = struct.pack(
        "<IIi", tl.DecryptedMessageActionReadMessages.CONSTRUCTOR_ID, 0x1CB5C415, count
    )
    with BinaryReader(data) as reader, pytest.raises((ValueError, BufferError, struct.error)):
        tl.read_object(reader)


def test_generated_parser_does_not_repair_invalid_utf8():
    data = bytes(tl.DecryptedMessage(random_id=1, ttl=0, message="x"))
    data = data[:-4] + b"\x01\xff\x00\x00"
    with BinaryReader(data) as reader, pytest.raises(UnicodeDecodeError):
        tl.read_object(reader)


@pytest.mark.parametrize(
    "mime", ["audio/ogg-invalid", "audio/opus-invalid", "audio/x-opus-invalid"]
)
def test_voice_mime_is_exact_not_a_prefix(mime):
    with pytest.raises(ValueError):
        files.resolve_kind("voice_note", file_name="voice.ogg", mime_type=mime)


def test_visual_hash_is_not_wire_fingerprint_and_survives_rekey():
    manager, chat = ready_manager()
    expected = hashlib.sha1(KEY).digest()[:16] + hashlib.sha256(KEY).digest()[:20]
    assert chat.key_hash == expected
    rekey.adopt_new_key(chat, OTHER)
    assert chat.key_hash == expected
    manager._save(chat)
    assert manager._storage.load(chat.id)["initial_key_hash"] == expected


def test_service_event_repr_never_includes_action_repr():
    class Sensitive:
        def __repr__(self):
            return "SYNTHETIC-DO-NOT-PRINT"

    assert "SYNTHETIC-DO-NOT-PRINT" not in repr(ServiceActionReceived(1, "action", Sensitive()))
