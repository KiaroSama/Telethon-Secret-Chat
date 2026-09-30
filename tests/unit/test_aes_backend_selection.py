"""CI must execute the AES backend named by its matrix leg, not just install it."""

import os

from telethon.crypto import aes


def test_ci_aes_backend_matches_its_declared_matrix_leg():
    expected = os.environ.get("TSC_TEST_AES_BACKEND")
    if expected is None:
        # Local runs may use either backend; both are supported.
        return
    assert expected in {"default", "cryptg"}
    assert (aes.cryptg is not None) == (expected == "cryptg")
