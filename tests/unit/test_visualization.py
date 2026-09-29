"""The key picture an official client draws (spec 005, US4).

Rule: TDLib td_api.tl `secretChat.key_hash` (tdlib/td 42e6a52, line 2813) with the Android client's
bit order (`IdenticonDrawable.java`); by-eye MATCH on the official Android client 2026-09-29
(docs/design/key-visualization.md).
"""

import ast
import sys
from pathlib import Path

import pytest

from telethon_secret_chat import PALETTE, KeyVisualization, key_visualization

EXPECTED_ROWS = """
000010002000
300001001100
210031000200
120022003200
030013002300
330000101010
201030100110
111021103110
021012102210
321003101310
231033100020
102020203020
""".split()


def test_the_design_vector():
    picture = key_visualization(bytes(range(36)))
    assert isinstance(picture, KeyVisualization)
    assert ["".join(map(str, row)) for row in picture.rows] == EXPECTED_ROWS
    assert picture.hex == bytes(range(32)).hex()


def test_the_palette_is_the_documented_one():
    assert PALETTE == ("#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9")


@pytest.mark.parametrize("value", [bytes(35), bytes(37), None, "0" * 36])
def test_anything_but_36_bytes_is_refused(value):
    with pytest.raises(ValueError):
        key_visualization(value)


def test_the_module_needs_only_the_standard_library():
    source = Path(__file__).parents[2] / "telethon_secret_chat" / "visualization.py"
    tree = ast.parse(source.read_text(encoding="utf-8"))
    names = {
        alias.name.split(".")[0]
        for node in ast.walk(tree)
        if isinstance(node, (ast.Import, ast.ImportFrom))
        for alias in (
            node.names if isinstance(node, ast.Import) else [ast.alias(node.module or "")]
        )
    }
    assert names <= set(sys.stdlib_module_names) | {"__future__"}
