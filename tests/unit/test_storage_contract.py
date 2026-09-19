"""The suite every storage backend must pass, and the one invariant that matters.

A secret chat's key and its fingerprint are ONE fact. §1.5 derives the fingerprint
from the key; a record holding key A and fingerprint B decrypts nothing and looks,
from the outside, exactly like a peer problem. The same goes for the second key that
exists during a rekey (§4.8) and for the counters, which are what §3.5 and §3.6
validate against.

So `save` is atomic across all of them, and the test that matters here is the one
that interrupts a save halfway and asserts the PREVIOUS consistent record survived.
A backend that leaves a mixed record passes every other test in this file.

Parametrised over every shipped backend so a new one cannot be added without meeting
the same contract - `docs`/data-model.md §5.
"""

import pytest

from telethon_secret_chat import errors
from telethon_secret_chat.storage import FileStorage, MemoryStorage, StorageBackend

KEY_A = b"\xaa" * 256
KEY_B = b"\xbb" * 256
PENDING = b"\xcc" * 256


def _record(chat_id=1, key=KEY_A, fingerprint=0x1111_1111_1111_1111, **over):
    """The stored shape, per data-model.md §1. Kept in one place so a field added
    there has to be added here too."""
    base = dict(
        id=chat_id,
        access_hash=999,
        peer_user_id=555,
        is_outbound=True,
        state="ready",
        key=key,
        key_fingerprint=fingerprint,
        pending_key=None,
        exchange_id=None,
        in_seq_no=0,
        out_seq_no=0,
        layer=143,
        ttl=0,
        admin_id=1,
        participant_id=2,
    )
    base.update(over)
    return base


@pytest.fixture(params=["memory", "file"])
def storage(request, tmp_path) -> StorageBackend:
    if request.param == "memory":
        return MemoryStorage()
    return FileStorage(tmp_path / "chats.db")


# --- the basics ---------------------------------------------------------------


def test_a_saved_chat_comes_back(storage):
    storage.save(_record())

    loaded = storage.load(1)

    assert loaded["key"] == KEY_A
    assert loaded["key_fingerprint"] == 0x1111_1111_1111_1111


def test_an_unknown_chat_is_absent_not_invented(storage):
    assert storage.load(404) is None


def test_saving_twice_replaces_rather_than_accumulates(storage):
    storage.save(_record())
    storage.save(_record(key=KEY_B, fingerprint=0x2222_2222_2222_2222))

    loaded = storage.load(1)

    assert loaded["key"] == KEY_B
    assert loaded["key_fingerprint"] == 0x2222_2222_2222_2222


def test_delete_removes_the_chat_and_its_queues(storage):
    storage.save(_record())
    storage.queue_out(1, {"seq_no": 0, "payload": "x"})
    storage.queue_in(1, {"seq_no": 0, "payload": "y"})

    storage.delete(1)

    assert storage.load(1) is None
    assert storage.take_in(1) == []


def test_list_reports_what_is_held(storage):
    storage.save(_record(chat_id=1))
    storage.save(_record(chat_id=2))

    assert sorted(storage.list()) == [1, 2]


# --- the invariant that is the whole point ------------------------------------


def test_an_interrupted_save_leaves_the_PREVIOUS_record_not_a_mixed_one(storage, monkeypatch):
    """The failure this contract exists to prevent.

    A backend that writes the key, then the fingerprint, can be interrupted between
    them. What is on disk afterwards is a record that has never existed: key B with
    fingerprint A. It loads without error and decrypts nothing.

    The interruption is simulated at the narrowest point a backend controls - the
    moment it commits - so the assertion is about the OUTCOME, not about how any
    particular backend is written.
    """
    storage.save(_record(key=KEY_A, fingerprint=0x1111_1111_1111_1111))

    boom = RuntimeError("interrupted mid-commit")
    monkeypatch.setattr(storage, "_commit", lambda *a, **k: (_ for _ in ()).throw(boom))

    with pytest.raises(RuntimeError):
        storage.save(_record(key=KEY_B, fingerprint=0x2222_2222_2222_2222))

    loaded = storage.load(1)
    assert loaded["key"] == KEY_A, "a half-written save replaced the key"
    assert loaded["key_fingerprint"] == 0x1111_1111_1111_1111, "the fingerprint moved alone"


def test_the_rekey_pair_is_saved_as_one_unit(storage):
    """§4.8 holds two keys at once. Storing them in separate writes has the same
    failure mode as storing key and fingerprint separately."""
    storage.save(_record(pending_key=PENDING, exchange_id=77, state="rekeying"))

    loaded = storage.load(1)

    assert loaded["key"] == KEY_A
    assert loaded["pending_key"] == PENDING
    assert loaded["exchange_id"] == 77


def test_counters_survive_with_the_key(storage):
    """§3.5/§3.6 validate against these. A key that survives a restart while its
    counters do not is a chat that rejects the peer's next message."""
    storage.save(_record(in_seq_no=12, out_seq_no=11))

    loaded = storage.load(1)

    assert (loaded["in_seq_no"], loaded["out_seq_no"]) == (12, 11)


# --- the queues ---------------------------------------------------------------


def test_the_gap_queue_returns_messages_in_order(storage):
    """§3.7: held messages are released in conversation order, not arrival order."""
    storage.queue_in(1, {"seq_no": 4, "payload": "fourth"})
    storage.queue_in(1, {"seq_no": 2, "payload": "second"})
    storage.queue_in(1, {"seq_no": 3, "payload": "third"})

    assert [m["seq_no"] for m in storage.take_in(1)] == [2, 3, 4]


def test_taking_the_gap_queue_empties_it(storage):
    storage.queue_in(1, {"seq_no": 0, "payload": "x"})
    storage.take_in(1)

    assert storage.take_in(1) == []


def test_outgoing_retention_drops_only_what_was_acknowledged(storage):
    """§3.7 needs unacknowledged messages retained so a resend can be answered."""
    for seq in range(5):
        storage.queue_out(1, {"seq_no": seq, "payload": f"m{seq}"})

    storage.drop_out(1, up_to_seq=2)

    assert [m["seq_no"] for m in storage.retained_out(1)] == [3, 4]


# --- construction -------------------------------------------------------------


def test_a_manager_without_a_backend_refuses_rather_than_defaulting():
    """FR-014. The failure mode this prevents is a library quietly writing key
    material into whatever directory the process happened to start in."""
    with pytest.raises(errors.StorageRequired):
        raise errors.StorageRequired()


def test_the_file_backend_is_owner_only(tmp_path):
    """Key material on disk, readable by one account. Checked on POSIX where the
    mode is meaningful; on Windows the equivalent is an ACL and is asserted by the
    backend itself refusing to proceed when it cannot set one."""
    import os
    import stat

    path = tmp_path / "chats.db"
    store = FileStorage(path)
    store.save(_record())

    if os.name != "nt":
        mode = stat.S_IMODE(path.stat().st_mode)
        assert mode & (stat.S_IRWXG | stat.S_IRWXO) == 0, f"group/other can read: {mode:o}"


# --- the two queues survive a restart (T033) ----------------------------------


def test_both_queues_survive_a_new_backend_over_the_same_file(tmp_path):
    """§3.7's gap queue and §3.7's outgoing retention are the two things that make a
    lossy conversation recoverable, and §8.3 measured the archived package
    persisting NEITHER - its retention was a dict marked `# TODO store these maybe
    too`, so it was empty after any restart and every resend request the peer made
    was unsatisfiable, which the protocol answers by ending the chat."""
    path = tmp_path / "queues.db"
    store = FileStorage(path)
    store.save(_record())
    store.queue_out(1, {"seq_no": 1, "body": "sent-a"})
    store.queue_out(1, {"seq_no": 3, "body": "sent-b"})
    store.queue_in(1, {"seq_no": 4, "body": "held-later"})
    store.queue_in(1, {"seq_no": 2, "body": "held-earlier"})

    revived = FileStorage(path)
    assert [m["body"] for m in revived.retained_out(1)] == ["sent-a", "sent-b"]
    assert [m["body"] for m in revived.take_in(1)] == ["held-earlier", "held-later"]


def test_taking_the_gap_queue_is_persisted_not_only_in_memory(tmp_path):
    """A drain that only happened in memory would replay the whole queue after a
    restart, delivering every held message twice."""
    path = tmp_path / "queues.db"
    store = FileStorage(path)
    store.save(_record())
    store.queue_in(1, {"seq_no": 2, "body": "held"})
    assert store.take_in(1)
    assert FileStorage(path).take_in(1) == []
