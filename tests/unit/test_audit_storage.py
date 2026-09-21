"""Regression tests for transactional storage, isolation, and failed writes."""

from copy import deepcopy

import pytest

from telethon_secret_chat.storage import FileStorage, MemoryStorage


@pytest.fixture(params=[MemoryStorage, FileStorage], ids=["memory", "file"])
def store(request, tmp_path):
    return request.param() if request.param is MemoryStorage else request.param(tmp_path / "s.db")


def test_records_and_retention_do_not_alias_callers(store):
    record = {"id": 1, "nested": {"items": [1]}}
    store.save(record)
    record["nested"]["items"].append(2)
    loaded = store.load(1)
    loaded["nested"]["items"].append(3)
    assert store.load(1)["nested"]["items"] == [1]
    message = {"seq_no": 1, "metadata": {"ids": [7]}}
    store.queue_out(1, message)
    message["metadata"]["ids"].append(8)
    store.retained_out(1)[0]["metadata"]["ids"].append(9)
    assert store.retained_out(1)[0]["metadata"]["ids"] == [7]


def test_an_outgoing_sequence_has_one_retained_record(store):
    store.queue_out(1, {"seq_no": 1, "body": "old", "pending": True})
    store.queue_out(1, {"seq_no": 1, "body": "old", "pending": False})
    assert store.retained_out(1) == [{"seq_no": 1, "body": "old", "pending": False}]


def test_duplicate_gap_frames_do_not_grow_the_queue(store):
    for _ in range(20):
        store.queue_in(1, {"seq_no": 3, "body": "first"})
    store.queue_in(1, {"seq_no": 3, "body": "replacement"})
    assert store.take_in(1) == [{"seq_no": 3, "body": "first"}]


@pytest.mark.parametrize("operation", ["delete", "queue_out", "drop_out", "queue_in", "take_in"])
def test_every_file_mutation_rolls_back_on_write_failure(tmp_path, monkeypatch, operation):
    store = FileStorage(tmp_path / "s.db")
    store.save({"id": 1, "counter": 0})
    store.queue_out(1, {"seq_no": 1, "body": "original"})
    store.queue_in(1, {"seq_no": 2, "body": "waiting"})
    before = deepcopy(store._state)
    disk = store.path.read_bytes()

    def fail():
        raise OSError("injected write failure")

    with monkeypatch.context() as patch:
        patch.setattr(store, "_write", fail)
        with pytest.raises(OSError):
            if operation == "delete":
                store.delete(1)
            elif operation == "queue_out":
                store.queue_out(1, {"seq_no": 3, "body": "new"})
            elif operation == "drop_out":
                store.drop_out(1, 1)
            elif operation == "queue_in":
                store.queue_in(1, {"seq_no": 4, "body": "new"})
            else:
                store.take_in(1)
    assert store._state == before
    assert store.path.read_bytes() == disk
    store.save({"id": 1, "counter": 1})
    assert FileStorage(store.path).retained_out(1) == [{"seq_no": 1, "body": "original"}]
    assert store.take_in(1) == [{"seq_no": 2, "body": "waiting"}]


def test_record_and_queues_commit_as_one_transaction(store):
    store.save({"id": 1, "counter": 0})
    with pytest.raises(RuntimeError, match="injected"):
        with store.transaction():
            store.queue_out(1, {"seq_no": 1, "body": "never sent"})
            store.queue_in(1, {"seq_no": 2, "body": "not accepted"})
            store.save({"id": 1, "counter": 1})
            raise RuntimeError("injected")
    assert store.load(1) == {"id": 1, "counter": 0}
    assert store.retained_out(1) == []
    assert store.take_in(1) == []


def test_multi_operation_file_transaction_performs_one_replace(tmp_path, monkeypatch):
    store = FileStorage(tmp_path / "s.db")
    calls = []
    original = store._write

    def counted():
        calls.append(1)
        original()

    monkeypatch.setattr(store, "_write", counted)
    with store.transaction():
        store.queue_out(1, {"seq_no": 1, "body": "frame"})
        store.save({"id": 1, "counter": 1})
    assert calls == [1]
    reopened = FileStorage(store.path)
    assert reopened.load(1)["counter"] == 1
    assert reopened.retained_out(1)[0]["seq_no"] == 1
