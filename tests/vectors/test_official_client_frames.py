"""Frames an official Telegram client wrote, replayed offline.

Constitution Principle I wants cryptographic behaviour pinned by something this
project did not write. ``test_kdf_matches_telethon.py`` covers the KDF against
Telethon; this file covers the rest of the receive path - frame decryption, the
wrapper, the ``seq_no`` transform and layer handling - against bytes an official
client produced in a live interop run.

The fixtures come from the interop tier's capture mode (``TSC_CAPTURE_DIR``, see
the README) and follow one provenance rule: a throwaway chat, closed before the file
was written, so the key inside protected nothing that still exists. They carry no
chat id, account, username or session. A fixture is regenerated from a new
throwaway chat, never edited by hand.

No fixture in the tree is a skip with its reason, not a failure: the capture needs
the operator and a real account.
"""

import json
from pathlib import Path

import pytest

from telethon_secret_chat import crypto, framing, sequence
from telethon_secret_chat.chat import SecretChat
from telethon_secret_chat.storage import MemoryStorage

FIXTURES = sorted((Path(__file__).parent / "fixtures").glob("official-client-*.json"))


def _replay(document):
    key = bytes.fromhex(document["key"])
    chat = SecretChat(id=1, access_hash=0, peer_user_id=0, is_outbound=document["is_outbound"])
    chat.adopt_key(key)
    # The peer echoes this side's counter; the replay has to have sent as much.
    chat.out_seq_no = document["local_out_seq_no"]
    storage = MemoryStorage()
    delivered = []
    for entry in document["frames"]:
        wrapper = framing.unwrap(
            crypto.decrypt_frame(key, bytes.fromhex(entry["frame"]), chat.in_x)
        )
        expected = entry["expected"]
        inner = wrapper.message
        assert (wrapper.layer, wrapper.in_seq_no, wrapper.out_seq_no) == (
            expected["layer"],
            expected["in_seq_no"],
            expected["out_seq_no"],
        )
        assert type(inner).__name__ == expected["type"]
        assert getattr(inner, "message", None) == expected["text"]
        delivered.extend(sequence.accept(chat, wrapper, storage).ready)
    return chat, delivered


def test_official_client_frames_replay_offline():
    if not FIXTURES:
        pytest.skip(
            "no official-client capture in tests/vectors/fixtures; one interop run with "
            "TSC_CAPTURE_DIR set produces it (README, Running the tests)"
        )
    for path in FIXTURES:
        document = json.loads(path.read_text(encoding="utf-8"))
        chat, delivered = _replay(document)
        # Every frame the peer sent was accepted in order, with no hole left behind.
        assert len(delivered) == len(document["frames"]) == chat.in_seq_no, path.name
        assert any(
            document["reply_contains"] in (getattr(one.message, "message", "") or "")
            for one in delivered
        ), f"{path.name}: the operator's reply is not among the replayed frames"
