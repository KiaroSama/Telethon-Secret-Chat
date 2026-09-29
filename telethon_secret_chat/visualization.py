"""The key picture a person compares on two screens (``ChatReady.key_hash``).

Rule: TDLib ``td_api.tl`` ``secretChat.key_hash`` (tdlib/td 42e6a52, line 2813): the 36 bytes
split into 2-bit pixels of four colours, a 12x12 square filled left to right, top to bottom, or
the first 32 bytes as hex. "Little-endian" is settled by the official Android client
(``IdenticonDrawable.java``, DrKLO/Telegram dc780e8): pixel ``n`` is
``(data[2n // 8] >> (2n % 8)) & 3``, the least significant pair of a byte first. Checked by eye
against the Android client's Encryption Key screen on 2026-09-29 (docs/design/key-visualization.md).
Drawing is the application's; this module holds the rule only.
"""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["PALETTE", "KeyVisualization", "key_visualization"]

#: Index = the 2-bit pixel value, in ``td_api.tl``'s order.
PALETTE = ("#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9")

_SIDE = 12
_LENGTH = 36


@dataclass(frozen=True)
class KeyVisualization:
    #: 12 rows of 12 palette indexes, top to bottom.
    rows: tuple
    #: The first 32 bytes, 64 lowercase hex digits.
    hex: str


def key_visualization(key_hash: bytes) -> KeyVisualization:
    """The grid and hex for a chat's key visualization value.

    Raises: ValueError unless given exactly 36 bytes (a legacy chat has none; the caller
    decides what to show then).
    """
    if not isinstance(key_hash, (bytes, bytearray)) or len(key_hash) != _LENGTH:
        raise ValueError("a key visualization value is exactly 36 bytes")
    pixels = [(key_hash[2 * n // 8] >> (2 * n % 8)) & 3 for n in range(_SIDE * _SIDE)]
    rows = tuple(tuple(pixels[r * _SIDE : (r + 1) * _SIDE]) for r in range(_SIDE))
    return KeyVisualization(rows=rows, hex=bytes(key_hash[:32]).hex())
