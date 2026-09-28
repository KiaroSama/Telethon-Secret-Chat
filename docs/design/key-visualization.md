# Design note: rendering the key visualization

Spike of plan 042, 2026-09-29. Output: the rendering rule with its sources, the API for a
follow-up feature, a fixed test vector, and the acceptance criteria. No code shipped.

## The input

`SecretChat.initial_key_hash` (`chat.py`, `adopt_key`) is `sha1(key)[:16] + sha256(key)[:20]`
of the chat's FIRST shared key, computed once and never after a rekey; it reaches
applications as `ChatReady.key_hash` and, after plan 023, `ChatSnapshot.key_hash`. TDLib
computes the same bytes the same way, and only on the initial exchange:
`SecretChatActor::calc_key_hash()` takes `sha1(auth_key)[:16] + sha256(auth_key)[:20]`, called
from the two handshake paths only (`td/telegram/SecretChatActor.cpp`, tdlib/td commit
`42e6a5259551178d1dab54a22ad96d14bd906e20`, 2026-09-25, lines 1909-1919, 517, 1780).

## The rule (quoted)

TDLib's `td_api.tl` (same commit, line 2813), documentation of `secretChat.key_hash`:

> This is a string of 36 little-endian bytes, which must be split into groups of 2 bits, each
> denoting a pixel of one of 4 colors FFFFFF, D5E6F3, 2D5775, and 2F99C9. The pixels must be
> used to make a 12x12 square image filled from left to right, top to bottom. Alternatively,
> the first 32 bytes of the hash can be converted to the hexadecimal format and printed as 32
> 2-digit hex numbers

"Little-endian" leaves the bit order inside a byte to interpretation. The official Android
client settles it (`TMessagesProj/src/main/java/org/telegram/ui/Components/IdenticonDrawable.java`,
DrKLO/Telegram commit `dc780e81ed1261c369c27870e8e0999a1eb0b600`): pixel `n` takes
`(data[2n / 8] >> (2n % 8)) & 0x3`, i.e. the LEAST significant pair of each byte first, and
indexes the palette `{0xffffffff, 0xffd5e6f3, 0xff2d5775, 0xff2f99c9}` - the same four colours
in the same order as `td_api.tl`. The same client prints the hex as 32 two-digit groups,
four per group separated by a space and eight per line (`IdenticonActivity.java`), and shows
five emoji derived from bytes 16-35 with its own emoji table; the emoji are out of scope here
because the table is the client's, not the protocol's.

## API for the follow-up feature

A pure function in a new `telethon_secret_chat/visualization.py`, importing nothing from the
protocol modules, exported from the package:

```python
PALETTE = ("#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9")  # index = the 2-bit value

@dataclass(frozen=True)
class KeyVisualization:
    rows: tuple[tuple[int, ...], ...]  # 12 rows of 12 palette indices, top to bottom
    hex: str                           # the first 32 bytes, 64 lowercase hex digits

def key_visualization(key_hash: bytes) -> KeyVisualization: ...
```

It refuses anything but exactly 36 bytes with `ValueError` (a legacy chat's `key_hash` is
`None`; the caller decides what to show then). Drawing a PNG, SVG or emoji grid is the
application's. The palette is a documented constant, never "improved".

## Test vector (computed by the rule above)

Input: `bytes(range(36))`. Expected rows, top to bottom:

```
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
```

Expected hex: `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f`.

Check by hand: byte 1 (`0x01`) gives pairs 1, 0, 0, 0 and byte 2 (`0x02`) gives 2, 0, 0, 0, so
the first row reads `0000 1000 2000`.

## By-eye check (UNVERIFIED)

The rule is confirmed from two independent sources (TDLib's documentation and the Android
client's renderer), but not yet by eye against a live chat. The check, for the operator:

1. Run the round-trip interop test (`scripts/run_interop.ps1 -Only
   tests/interop/test_live_roundtrip.py::test_both_ends_agree_on_the_key_fingerprint`) with a
   breakpoint or a print of `ready.key_hash.hex()` before the chat is closed.
2. On the second account, open the chat's "Encryption key" screen in the official client.
3. Render the printed hash with the rule above (12 lines of 12 digits, or four symbols) and
   compare it with the picture, and the hex with the numbers printed under it.

Record MATCH or NO MATCH with the date in the local testing notes. NO MATCH with the
most-significant-pair-first order as well would be a STOP for the feature.

## Acceptance criteria for the feature

- Unit: the vector above; `ValueError` for 35 and 37 bytes and for `None`.
- The function has no dependency outside the standard library.
- Live: one recorded by-eye MATCH against an official client.
- README: the usage example draws from `key_visualization(e.key_hash)`.
