"""protocol-reference.md §6 - encrypted files, and the check before a byte is written.

§6.1: "All files sent to secret chats are encrypted with one-time keys that are in
no way related to the chat's shared key." Per file: a fresh 32-byte key and a fresh
32-byte IV, AES-IGE, no ``x``, no ``msg_key``.

§6.2 is the trap, and it is a DIFFERENT algorithm from §1.5's: ``digest = md5(key +
iv)``, then ``fingerprint = substr(digest, 0, 4) XOR substr(digest, 4, 4)``. MD5, not
SHA-1. 32 bits, not 64. It travels OUTSIDE the encrypted message, so it is what lets
a receiver check that the key it found inside belongs to the file attached outside.

§6.3: "A receiver MUST recompute §6.2 from the key/iv inside the message and compare
it against encryptedFile.key_fingerprint before decrypting; a mismatch is a
rejection, not a warning." The design says the check happens "before the content is
handed over or written anywhere", which is what the last group here asserts.
"""

import hashlib
import os

import pytest

from telethon_secret_chat import crypto, files
from telethon_secret_chat.errors import MessageRejected


def a_key_pair():
    return os.urandom(32), os.urandom(32)


# --- §6.1 one-time keys -------------------------------------------------------


def test_a_file_key_is_thirty_two_bytes_and_so_is_its_iv():
    """ "2 random 256-bit numbers are computed which will serve as the AES key and
    initialization vector"."""
    key, iv = files.new_file_key()
    assert len(key) == 32 and len(iv) == 32


def test_every_file_gets_its_own_key():
    assert len({files.new_file_key() for _ in range(50)}) == 50


def test_the_file_key_is_unrelated_to_the_chat_key():
    """ "in no way related to the chat's shared key". Nothing in the signature can
    take one, which is the strongest form of "unrelated" available."""
    import inspect

    assert not [
        p for p in inspect.signature(files.new_file_key).parameters if "chat" in p or "shared" in p
    ]


def test_the_file_key_comes_from_a_secure_source(monkeypatch):
    calls = []
    real = files.os.urandom
    monkeypatch.setattr(files.os, "urandom", lambda n: calls.append(n) or real(n))
    files.new_file_key()
    assert sum(calls) >= 64


# --- §6.2 the fingerprint -----------------------------------------------------


def test_the_fingerprint_is_the_documented_md5_fold():
    """``digest = md5(key + iv)``, ``fingerprint = digest[0:4] XOR digest[4:8]``."""
    key, iv = a_key_pair()
    digest = hashlib.md5(key + iv).digest()
    expected = int.from_bytes(
        bytes(a ^ b for a, b in zip(digest[0:4], digest[4:8])), "little", signed=True
    )
    assert files.file_fingerprint(key, iv) == expected


def test_it_is_md5_and_not_the_sha1_of_section_1_5():
    """The two fingerprints in this protocol are computed differently and are 32 and
    64 bits wide. Using §1.5's routine here produces a number that looks fine and
    that no official client agrees with."""
    key, iv = a_key_pair()
    assert files.file_fingerprint(key, iv) != crypto.key_fingerprint(key + iv)


def test_it_fits_a_signed_int32():
    """``encryptedFile.key_fingerprint`` is a TL ``int``."""
    for _ in range(50):
        key, iv = a_key_pair()
        assert -(2**31) <= files.file_fingerprint(key, iv) < 2**31


def test_a_different_key_or_iv_changes_it():
    key, iv = a_key_pair()
    assert files.file_fingerprint(key, iv) != files.file_fingerprint(key, os.urandom(32))
    assert files.file_fingerprint(key, iv) != files.file_fingerprint(os.urandom(32), iv)


# --- encryption ---------------------------------------------------------------


def test_a_file_round_trips_through_its_own_key():
    key, iv = files.new_file_key()
    content = os.urandom(1000)
    assert (
        files.decrypt_file(files.encrypt_file(content, key, iv), key, iv, len(content)) == content
    )


def test_the_ciphertext_is_padded_to_a_cipher_block():
    """AES-IGE has no partial block, so the stored file is padded - and the true
    length travels inside the message, which is why `decrypt_file` takes it."""
    key, iv = files.new_file_key()
    assert len(files.encrypt_file(b"x" * 1001, key, iv)) % 16 == 0


def test_the_padding_is_removed_on_the_way_back():
    key, iv = files.new_file_key()
    for size in (0, 1, 15, 16, 17, 4096):
        content = os.urandom(size)
        back = files.decrypt_file(files.encrypt_file(content, key, iv), key, iv, size)
        assert back == content, size


def test_a_file_encrypted_under_one_key_does_not_open_under_another():
    key, iv = files.new_file_key()
    other_key, other_iv = files.new_file_key()
    blob = files.encrypt_file(b"the contents" * 8, key, iv)
    assert files.decrypt_file(blob, other_key, other_iv, 96) != b"the contents" * 8


# --- the refusal before anything is written ----------------------------------


def test_a_mismatched_fingerprint_is_refused():
    """§6.3: "a mismatch is a rejection, not a warning"."""
    key, iv = a_key_pair()
    with pytest.raises(MessageRejected):
        files.verify_file_fingerprint(
            chat_id=1, key=key, iv=iv, claimed=files.file_fingerprint(key, iv) ^ 1
        )


def test_a_matching_fingerprint_is_accepted():
    key, iv = a_key_pair()
    files.verify_file_fingerprint(
        chat_id=1, key=key, iv=iv, claimed=files.file_fingerprint(key, iv)
    )


def test_nothing_is_written_when_the_fingerprint_does_not_match(tmp_path):
    """ "verifying a received file's key fingerprint BEFORE the content is
    handed over or written anywhere". "it is refused rather than
    written to disk"."""
    key, iv = a_key_pair()
    target = tmp_path / "should-not-exist.bin"
    with pytest.raises(MessageRejected):
        files.save(
            target,
            ciphertext=files.encrypt_file(b"secret bytes", key, iv),
            key=key,
            iv=iv,
            size=12,
            claimed_fingerprint=files.file_fingerprint(key, iv) ^ 0xFF,
            chat_id=1,
        )
    assert not target.exists(), "a file with a bad fingerprint reached the disk"
    assert list(tmp_path.iterdir()) == [], "a temporary file was left behind"


def test_a_good_file_is_written(tmp_path):
    key, iv = a_key_pair()
    target = tmp_path / "ok.bin"
    content = b"the real contents" * 10
    written = files.save(
        target,
        ciphertext=files.encrypt_file(content, key, iv),
        key=key,
        iv=iv,
        size=len(content),
        claimed_fingerprint=files.file_fingerprint(key, iv),
        chat_id=1,
    )
    assert written.read_bytes() == content


def test_the_refusal_names_no_key_and_no_content():
    """Principle IV. The file key is key material even though it is one-time, and
    the message it arrived in is plaintext."""
    key, iv = a_key_pair()
    with pytest.raises(MessageRejected) as caught:
        files.verify_file_fingerprint(chat_id=1, key=key, iv=iv, claimed=0)
    rendered = str(caught.value) + repr(caught.value)
    assert key.hex()[:16] not in rendered.lower()
    assert iv.hex()[:16] not in rendered.lower()


# --- the README's API reference: send_file and save_file -----------------------------


async def test_a_file_crosses_the_chat_and_opens_on_the_other_side(pair, tmp_path):
    """Sent, then saved, and the bytes match."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)

    source = tmp_path / "picture.bin"
    content = os.urandom(5000)
    source.write_bytes(content)
    await a.send_file(chat_a.id, source)

    assert got, "the file message never arrived"
    written = await b.save_file(got[-1], tmp_path / "received.bin")
    assert written.read_bytes() == content


async def test_the_server_only_ever_sees_ciphertext(pair, tmp_path):
    """§6.4: "the bytes are IGE-encrypted client-side BEFORE upload.saveFilePart, so
    the server stores ciphertext only"."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    source = tmp_path / "plain.bin"
    source.write_bytes(b"RECOGNISABLE" * 100)
    await a.send_file(chat_a.id, source)
    assert b"RECOGNISABLE" not in wire.a.uploaded[-1], "the plaintext reached the server"


async def test_the_key_travels_inside_and_the_fingerprint_outside(pair, tmp_path):
    """§6.3: "the key for direct decryption will be sent in the body of the message",
    while the address and the 32-bit fingerprint go outside it in cleartext."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    source = tmp_path / "f.bin"
    source.write_bytes(b"x" * 64)
    await a.send_file(chat_a.id, source)

    message = got[-1]
    assert len(message.media.key) == 32 and len(message.media.iv) == 32
    assert message.file.key_fingerprint == files.file_fingerprint(
        message.media.key, message.media.iv
    )


async def test_saving_a_file_whose_fingerprint_is_wrong_writes_nothing(pair, tmp_path):
    """Through the public surface."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    source = tmp_path / "f.bin"
    source.write_bytes(b"y" * 64)
    await a.send_file(chat_a.id, source)

    message = got[-1]
    message.file.key_fingerprint ^= 0xFF  # as a hostile server could have sent it
    target = tmp_path / "must-not-appear.bin"
    with pytest.raises(MessageRejected):
        await b.save_file(message, target)
    assert not target.exists()


async def test_sending_a_file_refuses_on_a_chat_that_is_not_ready(pair, tmp_path):
    wire, a, _ = pair
    from telethon_secret_chat.errors import ChatNotReady

    chat = await a.create(2000)
    source = tmp_path / "f.bin"
    source.write_bytes(b"z")
    with pytest.raises(ChatNotReady):
        await a.send_file(chat.id, source)


async def test_sending_an_unreadable_file_refuses(pair, tmp_path):
    """``send_file`` refuses an unreadable file."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    with pytest.raises(OSError):
        await a.send_file(chat_a.id, tmp_path / "does-not-exist.bin")


def _save_args(content=b"secret bytes"):
    key, iv = files.new_file_key()
    return dict(
        ciphertext=files.encrypt_file(content, key, iv),
        key=key,
        iv=iv,
        size=len(content),
        claimed_fingerprint=files.file_fingerprint(key, iv),
        chat_id=1,
    )


def test_a_leftover_media_temp_file_is_removed_before_saving(tmp_path):
    leftover = tmp_path / f".secret-chat-file-{os.getpid()}-x.tmp"
    leftover.write_bytes(b"plaintext of a file whose save was killed")
    files.save(tmp_path / "out.bin", **_save_args())
    assert not leftover.exists()
    assert (tmp_path / "out.bin").read_bytes() == b"secret bytes"


def test_an_interrupted_media_save_leaves_no_temp_file(tmp_path, monkeypatch):
    import os

    def fail(*args):
        raise OSError("synthetic fsync failure")

    monkeypatch.setattr(os, "fsync", fail)
    with pytest.raises(OSError):
        files.save(tmp_path / "out.bin", **_save_args())
    assert list(tmp_path.iterdir()) == []


# --- an upload does not make the chat deaf ------------------------------


async def test_a_message_arrives_while_a_file_is_uploading(pair, tmp_path):
    import asyncio

    from .fake_client import establish

    wire, a, b = pair
    chat_a, chat_b = await establish(a, b, wire)
    release = asyncio.Event()
    original = wire.a.upload_file

    async def held_upload(file, **kwargs):
        await asyncio.wait_for(release.wait(), timeout=5)
        return await original(file, **kwargs)

    wire.a.upload_file = held_upload
    got = []
    a.on("MessageReceived", lambda e: got.append(e.text))
    source = tmp_path / "big.bin"
    source.write_bytes(b"x" * 1000)
    upload = asyncio.ensure_future(a.send_file(chat_a.id, source))
    await asyncio.sleep(0)
    await b.send_message(chat_b.id, "while you upload")
    assert got == ["while you upload"], "the upload held the chat lock"
    release.set()
    assert isinstance(await asyncio.wait_for(upload, timeout=5), int)


async def test_a_text_sent_during_an_upload_lands_after_the_file(pair, tmp_path):
    import asyncio

    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    release = asyncio.Event()
    original = wire.a.upload_file

    async def held_upload(file, **kwargs):
        await asyncio.wait_for(release.wait(), timeout=5)
        return await original(file, **kwargs)

    wire.a.upload_file = held_upload
    order = []
    b.on("MessageReceived", lambda e: order.append("file" if e.media else e.text))
    source = tmp_path / "doc.bin"
    source.write_bytes(b"y" * 100)
    upload = asyncio.ensure_future(a.send_file(chat_a.id, source))
    await asyncio.sleep(0)
    text = asyncio.ensure_future(a.send_message(chat_a.id, "after the file"))
    await asyncio.sleep(0)
    assert not text.done()
    release.set()
    await asyncio.wait_for(asyncio.gather(upload, text), timeout=5)
    assert order == ["file", "after the file"]


# --- streamed IGE must equal the one-shot cipher byte for byte ----------


@pytest.mark.parametrize("size", [0, 1, 15, 16, 17, 1023, 1024, 1025, 150_007])
@pytest.mark.parametrize("piece", [1024, 64 * 1024])
def test_chunked_encryption_matches_the_one_shot_cipher(size, piece):
    import io
    import os

    from telethon.crypto import AES

    key, iv = files.new_file_key()
    data = os.urandom(size)
    pad = os.urandom(-size % 16)
    reader = files.EncryptingReader(io.BytesIO(data), key, iv, size, padding=pad)
    streamed = b"".join(iter(lambda: reader.read(piece), b""))
    assert reader.size == size + len(pad)
    assert streamed == AES.encrypt_ige(data + pad, key, iv)
    pieces = [streamed[i : i + 1000] for i in range(0, len(streamed), 1000)]
    assert b"".join(files.decrypt_stream(pieces, key, iv, size)) == data


async def test_a_multi_part_file_streams_both_ways_without_a_memory_copy(pair, tmp_path):
    """Parts are exact 512 KiB pieces, and the download lands on disk."""
    from .fake_client import establish

    wire, a, b = pair
    chat_a, _ = await establish(a, b, wire)
    got = []
    b.on("MessageReceived", got.append)
    source = tmp_path / "big.bin"
    content = os.urandom(3 * 512 * 1024 + 11)
    source.write_bytes(content)
    await a.send_file(chat_a.id, source)

    written = await b.save_file(got[-1], tmp_path / "out" / "big.bin")
    assert written.read_bytes() == content
    assert "BytesIO" not in b._client.download_targets
    assert list((tmp_path / "out").iterdir()) == [written]
