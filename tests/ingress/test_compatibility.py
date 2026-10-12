"""The optional prerequisite cannot silently change ordinary or transient contracts."""

import pytest
from telethon import TelegramClient
from telethon.sessions import MemorySession

import telethon_secret_chat as api
from unit.fake_client import FakeClient

pytestmark = pytest.mark.timeout(10)


def test_default_client_does_not_enable_ingress_and_public_modes_remain_distinct():
    client = TelegramClient(MemorySession(), 1, "0" * 32)
    assert client._durable_ingress is None
    assert api.SecretChatManager is not api.BoundedSecretChatManager
    assert api.StrongSecretChatManager is not api.TransientSecretChatManager
    assert api.TransientLimits().tasks == 8
    assert callable(api.BoundedSecretChatManager.receive_file)
    assert callable(api.StrongSecretChatManager.acknowledge_delivery)


def test_plain_or_volatile_backends_are_not_strong_custody(tmp_path):
    limits = api.IngressLimits(records=32, bytes=1024 * 1024, age_seconds=3600)
    for store in (api.MemoryStorage(), api.FileStorage(tmp_path / "plain.json")):
        with pytest.raises(api.IngressRefused):
            api.DurableIngress(store, limits=limits)
    assert FakeClient().handlers == []


async def test_unconfigured_caller_client_cannot_enable_strong_manager():
    with pytest.raises(api.IngressRefused):
        api.StrongSecretChatManager(
            FakeClient(),
            object(),
            limits=api.CustodyLimits(records=32, bytes=1024 * 1024, age_seconds=3600),
        )
