"""Protected persistence boundary; synthetic values never appear in assertion output."""

import json

import pytest

from telethon_secret_chat.chat import SecretChat

pytestmark = pytest.mark.timeout(10)


def test_protected_store_refuses_payload_before_any_commit(tmp_path):
    from telethon_secret_chat import ProtectedFileStorage, TransientRefused

    store = ProtectedFileStorage(tmp_path / "protected.json")
    before = store.path.read_bytes()
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    with pytest.raises(TransientRefused):
        store.save(chat.to_record())
    assert store.path.read_bytes() == before
    with pytest.raises(TransientRefused):
        store.queue_out(7, {"seq_no": 1, "body": "synthetic ordinary message"})
    assert store.path.read_bytes() == before
    assert json.loads(before)["out"] == {}


@pytest.mark.parametrize("operation", ["queue_in", "queue_out", "save_setting", "delete"])
def test_protected_store_rejects_every_content_side_channel(tmp_path, operation):
    from telethon_secret_chat import ProtectedFileStorage, TransientRefused

    store = ProtectedFileStorage(tmp_path / "protected.json")
    before = store.path.read_bytes()
    arguments = ("ordinary", "synthetic") if operation == "save_setting" else (7, {})
    if operation == "delete":
        arguments = (7,)
    with pytest.raises(TransientRefused):
        getattr(store, operation)(*arguments)
    assert store.path.read_bytes() == before


def test_protected_store_refuses_existing_default_even_when_empty(tmp_path):
    from telethon_secret_chat import FileStorage, ProtectedFileStorage, TransientRefused

    path = tmp_path / "ordinary.json"
    FileStorage(path)
    before = path.read_bytes()
    with pytest.raises(TransientRefused):
        ProtectedFileStorage(path)
    assert path.read_bytes() == before


def test_atomic_commit_failure_restores_ram_and_protected_state(tmp_path, monkeypatch):
    from telethon_secret_chat import ProtectedFileStorage, TransientLimits
    from telethon_secret_chat.transient_storage import TransientStorage

    protected = ProtectedFileStorage(tmp_path / "protected.json")
    ram = TransientStorage(protected, TransientLimits())
    ram.binding = {"user_id": 9, "auth_key_id": 10, "dc_id": 2}
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    ram.save(chat.to_record())
    before = protected.path.read_bytes()

    def fail():
        raise OSError("synthetic commit fault")

    monkeypatch.setattr(protected, "_write", fail)
    with pytest.raises(OSError):
        with ram.transaction():
            ram.queue_out(7, {"seq_no": 1, "body": "synthetic payload", "frame": "abcd"})
            chat.out_seq_no = 1
            ram.save(chat.to_record())
    assert ram.retained_out(7) == []
    assert ram.load(7)["out_seq_no"] == 0
    assert protected.path.read_bytes() == before


@pytest.mark.parametrize("extra", ["pending_deliveries", "unexpected"])
def test_protected_schema_rejects_extra_fields_without_serializing(tmp_path, extra):
    from telethon_secret_chat import ProtectedFileStorage, TransientRefused
    from telethon_secret_chat.protected import PROTECTED_FIELDS

    protected = ProtectedFileStorage(tmp_path / "protected.json")
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True).to_record()
    record = {name: chat[name] for name in PROTECTED_FIELDS}
    record.update(state="requested")
    envelope = {
        "format": 1,
        "binding": {"user_id": 9, "auth_key_id": 10, "dc_id": 2},
        "chat": record,
        "suspended": False,
    }
    record[extra] = "synthetic payload"
    before = protected.path.read_bytes()
    with pytest.raises(TransientRefused):
        protected.save(envelope)
    assert protected.path.read_bytes() == before


def test_ram_limits_fail_before_protected_commit(tmp_path):
    from telethon_secret_chat import ProtectedFileStorage, TransientLimits, TransientRefused
    from telethon_secret_chat.transient_storage import TransientStorage

    protected = ProtectedFileStorage(tmp_path / "protected.json")
    ram = TransientStorage(protected, TransientLimits(records=1))
    ram.binding = {"user_id": 9, "auth_key_id": 10, "dc_id": 2}
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    ram.save(chat.to_record())
    ram.queue_out(7, {"seq_no": 1, "body": "ab", "frame": "cd"})
    before = protected.path.read_bytes()
    with pytest.raises(TransientRefused):
        with ram.transaction():
            ram.queue_in(7, {"seq_no": 1, "body": "ef"})
            chat.in_seq_no = 1
            ram.save(chat.to_record())
    assert protected.path.read_bytes() == before
    assert ram.peek_in(7) == []
    assert len(ram.retained_out(7)) == 1


@pytest.mark.parametrize("budget", ["bytes", "attempts"])
def test_global_byte_and_attempt_bounds_refuse_without_persisting_payload(tmp_path, budget):
    from telethon_secret_chat import ProtectedFileStorage, TransientLimits, TransientRefused
    from telethon_secret_chat.transient_storage import TransientStorage

    protected = ProtectedFileStorage(tmp_path / "protected.json")
    limits = TransientLimits(bytes=64 * 1024, frame_bytes=16 * 1024, attempts=1)
    ram = TransientStorage(protected, limits)
    ram.binding = {"user_id": 9, "auth_key_id": 10, "dc_id": 2}
    chat = SecretChat(id=7, access_hash=8, peer_user_id=9, is_outbound=True)
    ram.save(chat.to_record())
    before = protected.path.read_bytes()
    if budget == "bytes":
        with pytest.raises(TransientRefused):
            ram.queue_out(7, {"seq_no": 1, "body": "x" * 20000, "frame": "abcd"})
        assert ram.retained_out(7) == []
    else:
        ram.queue_out(7, {"seq_no": 1, "body": "ab", "frame": "cd"})
        ram.attempt(7, 1)
        with pytest.raises(TransientRefused):
            ram.attempt(7, 1)
    assert protected.path.read_bytes() == before
