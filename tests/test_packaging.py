"""The one thing worth asserting before there is protocol code: that the licence
conditions this project inherited are actually met.

Principle V is not decoration. The base package is MIT and TDLib is Boost, and both
licences require their notice to travel with the work. A notice file is easy to write
once and easy to lose in a refactor, so it is pinned here rather than trusted.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_the_gpl_text_is_actually_present():
    """`license = "GPL-3.0-or-later"` in a manifest is a claim; LICENSE is the thing
    that makes it true."""
    licence = (ROOT / "LICENSE").read_text(encoding="utf-8")

    assert "GNU GENERAL PUBLIC LICENSE" in licence
    assert "Version 3, 29 June 2007" in licence


def test_the_inherited_notices_are_preserved():
    """Both upstream licences require their notice to be kept with the work.

    MIT: "The above copyright notice and this permission notice shall be included in
    all copies or substantial portions of the Software."
    Boost: "The copyright notices in the Software and this entire statement ... must
    be included in all copies of the Software."
    """
    notice = (ROOT / "NOTICE").read_text(encoding="utf-8")

    assert "MIT License" in notice, "painor's MIT notice was dropped"
    assert "Copyright (c) 2020 painor" in notice, "the MIT copyright line was dropped"
    assert "Boost Software License" in notice, "TDLib's Boost notice was dropped"
    assert "core.telegram.org/api/end-to-end" in notice, "the protocol source is unattributed"


def test_the_package_makes_no_promise_it_cannot_keep():
    """It imports, and it exports nothing. A stub that exported names would let a
    caller write code against an implementation that does not exist."""
    import telethon_secret_chat as pkg

    assert pkg.__all__ == []
