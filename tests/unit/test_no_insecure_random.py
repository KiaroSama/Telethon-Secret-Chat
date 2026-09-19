"""The constitution's randomness rule, read off the package's own source.

"Randomness MUST come from ``secrets`` or ``os.urandom``. ``random`` MUST NOT appear
in any path producing key material, nonces, or padding" (Secret Material Handling).

§8.8 measured three uses of it in the code this package replaces: the ``exchange_id``
of a rekey (``random.randint``, and only ~27 bits of an int64 field), and the length
of ``random_bytes`` on both the send and the notify paths. None of them looks wrong
in a diff, and none of them fails a test that does not go looking.

This file goes looking. It is separate from the tests for padding and rekeying
because the rule is not about either of them - it is about every line in the
package, including the ones not written yet.
"""

import ast
import pathlib

import pytest

import telethon_secret_chat

PACKAGE = pathlib.Path(telethon_secret_chat.__file__).parent
SOURCES = sorted(PACKAGE.rglob("*.py"))


def test_there_are_sources_to_check():
    """A glob that matched nothing would make every test below vacuously true."""
    assert len(SOURCES) > 10


@pytest.mark.parametrize("path", SOURCES, ids=lambda p: p.name)
def test_no_module_imports_random(path):
    """Read off the AST rather than a grep: ``import random``, ``from random import``
    and ``importlib.import_module("random")`` all count, while the word appearing in
    a comment or a docstring does not - and this file's own docstring is proof that
    a grep would produce false positives."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    offences = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            offences += [
                a.name for a in node.names if a.name == "random" or a.name.startswith("random.")
            ]
        elif isinstance(node, ast.ImportFrom) and node.module == "random":
            offences.append("from random import ...")
        elif isinstance(node, ast.Call):
            target = getattr(node.func, "attr", getattr(node.func, "id", ""))
            if target == "import_module" and node.args:
                if getattr(node.args[0], "value", None) == "random":
                    offences.append('import_module("random")')
    assert not offences, f"{path.name} reaches for the `random` module: {offences}"


def test_the_generated_schema_is_checked_too():
    """It is exempt from the line ceiling, not from the rules."""
    assert PACKAGE / "schema" / "secret_tl.py" in SOURCES


def test_the_secure_sources_are_the_ones_actually_used():
    """The ban is only half the rule. The other half is that key material, nonces and
    padding come from somewhere - so the modules that produce them are named here,
    and a module that stopped calling a CSPRNG would fail this even while passing the
    test above."""
    expected = {
        "crypto.py": "os",  # §2.2 padding
        "framing.py": "os",  # §3.3 random_bytes
        "files.py": "os",  # §6.1 the one-time file key
        "dh.py": "secrets",  # §1.2's Miller-Rabin bases
        "handshake.py": "secrets",  # §1.3 the private exponent
        "rekey.py": "secrets",  # §4.2 exchange_id
    }
    for name, module in expected.items():
        source = (PACKAGE / name).read_text(encoding="utf-8")
        tree = ast.parse(source, filename=name)
        imported = {
            alias.name
            for node in ast.walk(tree)
            if isinstance(node, ast.Import)
            for alias in node.names
        }
        assert module in imported, f"{name} no longer imports {module}"
