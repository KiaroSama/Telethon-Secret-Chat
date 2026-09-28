"""The constitution's 800-line ceiling, over every Python file this repository writes.

Test files and tools count too: a ceiling that only watched the package let a test
module drift to 661 lines unnoticed.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCANNED = ("telethon_secret_chat", "tests", "tools")


def test_no_file_exceeds_the_line_ceiling():
    """The one exemption is the constitution's own: "Generated schema files are
    exempt and MUST be marked as generated"."""
    over = []
    for folder in SCANNED:
        for path in sorted((ROOT / folder).rglob("*.py")):
            lines = path.read_text(encoding="utf-8").splitlines()
            if lines and lines[0].strip() == "# generated":
                continue
            if len(lines) > 800:
                over.append(f"{path.relative_to(ROOT)}: {len(lines)}")
    assert not over, over
