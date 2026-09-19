"""The interop tier needs two real Telegram accounts, so it skips by default.

It skips with the REASON stated rather than silently: a tier that quietly collects
zero tests looks exactly like a tier that passed, and this is the tier carrying the
only evidence that an official client can read what this package writes (SC-001).
"""

import os

import pytest

REQUIRED = ("TSC_TEST_SESSION", "TSC_TEST_PEER")


def pytest_collection_modifyitems(config, items):
    missing = [name for name in REQUIRED if not os.environ.get(name)]
    if not missing:
        return
    skip = pytest.mark.skip(
        reason=(
            "interop needs a real account: "
            + ", ".join(missing)
            + " unset. This tier is never run in CI - a session string in a CI secret "
            "is a credential one misconfigured log away from being public."
        )
    )
    for item in items:
        item.add_marker(skip)
