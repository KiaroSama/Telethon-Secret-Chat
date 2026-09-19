"""FR-016: the Telethon surface this package depends on, in one place.

"Telethon internals are a compatibility surface: every private attribute the package
touches MUST be listed in one place and covered by a test that fails when it
disappears" (constitution, Development Workflow).

research.md Q3 measured that surface and found it is **one** private attribute -
``TelegramClient._parse_message_text`` - against the five §8.0 counted in the
archived package. The rest of what this package needs from Telethon is public: the
TL requests, the TL types, and ``telethon.crypto.AES``.

When one of these fails, the failure message is the fix. That is the point of the
file: the fallback is decided once, here, rather than re-derived under pressure by
whoever is on call the day a Telethon release lands.
"""

import inspect

import telethon
from telethon import TelegramClient
from telethon.crypto import AES
from telethon.extensions import BinaryReader
from telethon.network.mtprotostate import MTProtoState
from telethon.tl import functions, types

# --- the one private attribute ------------------------------------------------


def test_parse_message_text_still_exists():
    """The ONE private attribute used at runtime.

    FALLBACK, if this fails: stop calling it and accept pre-parsed ``entities`` from
    the caller instead - ``manager._parse_text`` already returns ``(text, None)``
    when the attribute is absent, so the failure is a documentation change and a
    release note, not a code change. Re-implementing Markdown and HTML parsing here
    is not this package's business (research.md Q3).
    """
    assert hasattr(TelegramClient, "_parse_message_text"), (
        "Telethon removed TelegramClient._parse_message_text. The package already "
        "falls back to sending text unparsed and taking `entities` from the caller "
        "(manager._parse_text); say so in the release notes and delete this test."
    )


def test_the_package_survives_that_attribute_disappearing():
    """The fallback is not a plan, it is a code path - so it is exercised."""
    from telethon_secret_chat import SecretChatManager
    from telethon_secret_chat.storage import MemoryStorage

    class WithoutIt:
        def add_event_handler(self, *a, **k):
            pass

        def remove_event_handler(self, *a, **k):
            pass

    manager = SecretChatManager(WithoutIt(), storage=MemoryStorage())
    assert manager._parse_text("**bold**") == ("**bold**", None)


# --- the public surface -------------------------------------------------------


def test_aes_ige_is_still_public():
    """research.md Q1: ``telethon.crypto.AES`` is PUBLIC - not underscore-prefixed -
    so using it adds nothing to the private surface, and it is the same primitive
    Telethon's own transport runs on.

    FALLBACK, if this fails: there is none that keeps the constitution's promise.
    Writing an IGE loop here is writing the cryptographic code Phase 0 decided not
    to write; the answer is to pin the Telethon version and open an issue.
    """
    assert hasattr(AES, "encrypt_ige") and hasattr(AES, "decrypt_ige")
    assert inspect.signature(AES.encrypt_ige).parameters.keys() >= {"plain_text", "key", "iv"}


def test_the_binary_reader_is_still_public():
    """Used by the generated schema to read nested objects.

    FALLBACK: read the primitives directly with ``struct`` - they are four
    documented byte layouts, and the generated module already writes them by hand.
    """
    assert hasattr(BinaryReader, "read_int") and hasattr(BinaryReader, "tgread_bytes")


def test_every_tl_request_the_package_sends_still_exists():
    """FALLBACK: none needed - a TL request that vanishes means Telegram changed the
    API, and the schema has to be re-read either way."""
    for name in (
        "GetDhConfigRequest",
        "RequestEncryptionRequest",
        "AcceptEncryptionRequest",
        "DiscardEncryptionRequest",
        "SendEncryptedRequest",
        "SendEncryptedFileRequest",
    ):
        assert hasattr(functions.messages, name), name


def test_every_tl_type_the_package_reads_still_exists():
    for name in (
        "UpdateEncryption",
        "UpdateNewEncryptedMessage",
        "EncryptedChat",
        "EncryptedChatRequested",
        "EncryptedChatWaiting",
        "EncryptedChatDiscarded",
        "EncryptedFile",
        "EncryptedFileEmpty",
        "InputEncryptedChat",
        "InputEncryptedFileUploaded",
        "InputEncryptedFileLocation",
    ):
        assert hasattr(types, name), name


# --- the test oracle, which is private and deliberately not used at runtime ----


def test_the_kdf_oracle_still_exists():
    """``MTProtoState._calc_key`` is private and is used ONLY as a test oracle
    (research.md Q2, oracle 2). That is the whole reason §2.4 is vendored: a private
    method can change on any release, and a cryptographic step that changes without
    notice is the failure this package exists to avoid.

    FALLBACK, if this fails: tests/vectors/test_kdf_matches_telethon.py loses one of
    its three oracles. The vendored KDF keeps working unchanged - nothing on the wire
    moves - and the recorded TDLib vectors of Phase 8 become the remaining evidence.
    """
    assert hasattr(MTProtoState, "_calc_key")
    assert set(inspect.signature(MTProtoState._calc_key).parameters) == {
        "auth_key",
        "msg_key",
        "client",
    }, "the oracle's signature moved; tests/vectors must be updated to match"


def test_the_supported_telethon_version_is_stated():
    """FR-016: "The package MUST state the Telethon version it supports"."""
    import tomllib
    from pathlib import Path

    import telethon_secret_chat

    root = Path(telethon_secret_chat.__file__).parent.parent
    manifest = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    floor = [d for d in manifest["project"]["dependencies"] if d.startswith("telethon")]
    assert floor, "pyproject does not state a Telethon dependency"
    assert ">=" in floor[0], f"the Telethon floor is not stated: {floor[0]}"
    assert tuple(int(p) for p in telethon.__version__.split(".")[:2]) >= (1, 45)
