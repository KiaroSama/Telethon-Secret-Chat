"""The interop tier needs two real Telegram accounts, so it skips by default.

It skips with the REASON stated rather than silently: a tier that quietly collects
zero tests looks exactly like a tier that passed, and this is the tier carrying the
only evidence that an official client can read what this package writes (SC-001).

``pytest_collection_modifyitems`` is a SESSION hook even when it lives in a
subdirectory's conftest - pytest hands it every item it collected, not the ones
under this folder. Filtering by path is therefore not tidiness, it is the whole
correctness of the hook: without it one unset environment variable skipped the
entire repository and a full run reported "42 skipped" with nothing passed, which
is the false green the constitution's Principle III names in so many words.
"""

import os
from pathlib import Path

import pytest

REQUIRED = ("TSC_TEST_SESSION", "TSC_TEST_PEER")

HERE = Path(__file__).parent


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
        if HERE == Path(item.path).parent or HERE in Path(item.path).parents:
            item.add_marker(skip)
