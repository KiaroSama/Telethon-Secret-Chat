"""protocol-reference.md §2.2 - the padding rule, and where its bytes come from.

Two mandatory constraints, quoted: "The byte array is padded with 12 to 1024 random
padding bytes to make its length divisible by 16 bytes." So a length in [12, 1024],
and ``(4 + payload + padding) % 16 == 0``.

And one constitutional constraint on top: "Randomness MUST come from ``secrets`` or
``os.urandom``. ``random`` MUST NOT appear in any path producing key material,
nonces, or padding." §8.3 measured the archived package choosing its padding length
with ``random.randint`` - the non-cryptographic module - which is why the last test
here reads the package's own source.
"""

import ast
import pathlib
import re

import pytest

import telethon_secret_chat
from telethon_secret_chat import crypto

PACKAGE = pathlib.Path(telethon_secret_chat.__file__).parent


def payloads():
    """Lengths that put the total at every offset modulo 16, plus the boundaries."""
    return [b"x" * n for n in (0, 1, 11, 12, 15, 16, 17, 31, 63, 100, 1000)]


@pytest.mark.parametrize("payload", payloads())
def test_the_total_is_divisible_by_sixteen(payload):
    """The cipher is AES-IGE, which has no partial block."""
    assert len(crypto.pad_payload(payload)) % 16 == 0


@pytest.mark.parametrize("payload", payloads())
def test_the_padding_is_at_least_twelve_bytes(payload):
    """MTProto 1.0 used 0-15; 2.0's floor of 12 is what the length prefix and the
    msg_key construction assume."""
    assert len(crypto.pad_payload(payload)) - len(payload) >= 12


@pytest.mark.parametrize("payload", payloads())
def test_the_padding_is_at_most_1024_bytes(payload):
    """The sender-side ceiling. Receive-side is deliberately unbounded - §2.2 marks
    it UNVERIFIED and the oracle itself exceeds 1024."""
    assert len(crypto.pad_payload(payload)) - len(payload) <= 1024


def test_the_padding_is_not_constant():
    """Padding that repeated would be padding an attacker could subtract. The
    payload is fixed, so any difference is in the padding."""
    seen = {crypto.pad_payload(b"same") for _ in range(50)}
    assert len(seen) > 40


def test_the_payload_is_returned_unchanged_at_the_front():
    payload = bytes(range(64))
    assert crypto.pad_payload(payload).startswith(payload)


def test_the_padding_source_is_os_urandom(monkeypatch):
    """Not "it looks random" - which module was called.

    A test asserting only that the bytes differ would pass against
    ``random.randbytes``, which is exactly the defect §8.3 found in the code this
    package replaces.
    """
    calls = []
    real = crypto.os.urandom
    monkeypatch.setattr(crypto.os, "urandom", lambda n: calls.append(n) or real(n))
    crypto.pad_payload(b"x" * 20)
    assert calls, "pad_payload produced padding without asking os.urandom for it"


# --- the ban, enforced against the source -------------------------------------


def module_sources():
    for path in sorted(PACKAGE.rglob("*.py")):
        yield path, path.read_text(encoding="utf-8")


def test_the_random_module_is_not_imported_anywhere_in_the_package():
    """The constitution's "Secret Material Handling" rule, read off the AST rather
    than off a grep: ``import random``, ``from random import ...`` and
    ``importlib.import_module("random")`` all count, and a comment mentioning the
    word does not."""
    offenders = []
    for path, source in module_sources():
        tree = ast.parse(source, filename=str(path))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                offenders += [
                    f"{path.name}: import {a.name}"
                    for a in node.names
                    if a.name == "random" or a.name.startswith("random.")
                ]
            elif isinstance(node, ast.ImportFrom):
                if node.module == "random":
                    offenders.append(f"{path.name}: from random import ...")
            elif isinstance(node, ast.Call):
                target = getattr(node.func, "attr", getattr(node.func, "id", ""))
                if target == "import_module" and node.args:
                    literal = getattr(node.args[0], "value", None)
                    if literal == "random":
                        offenders.append(f"{path.name}: import_module('random')")
    assert not offenders, offenders


def test_the_generated_schema_is_included_in_that_ban():
    """The generated file is exempt from the line ceiling, not from the rules."""
    names = {path.name for path, _ in module_sources()}
    assert "secret_tl.py" in names


def test_no_module_exceeds_the_line_ceiling():
    """The constitution's 800 lines, with the one exemption it names: "Generated
    schema files are exempt and MUST be marked as generated"."""
    over = []
    for path, source in module_sources():
        lines = source.splitlines()
        if lines and lines[0].strip() == "# generated":
            continue
        if len(lines) > 800:
            over.append(f"{path.relative_to(PACKAGE)}: {len(lines)}")
    assert not over, over


def test_the_exempt_file_says_it_is_generated_on_its_first_line():
    first = (PACKAGE / "schema" / "secret_tl.py").read_text(encoding="utf-8").splitlines()[0]
    assert re.match(r"#\s*generated", first)
