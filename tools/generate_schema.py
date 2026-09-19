"""Turn the published end-to-end TL schema into Python.

Input:  ``telethon_secret_chat/schema/end-to-end.tl`` - fetched verbatim from
        https://core.telegram.org/schema/end-to-end, the normative source under
        Constitution Principle II.
Output: ``telethon_secret_chat/schema/secret_tl.py`` - generated, and the one file
        exempt from the 800-line ceiling.

Run it with ``uv run --locked python tools/generate_schema.py`` after re-fetching
the schema. It is a build tool, not part of the shipped package.

Why generate rather than hand-write: a constructor id is the CRC32 of a
declaration, and a digit wrong in one of ninety of them is a message the peer
silently cannot parse. The published line is copied once; every id, every field
order and every flag position is derived from it mechanically.

Why our own classes rather than Telethon's: Telethon ships the OUTER encrypted
types (``EncryptedChat``, ``messages.sendEncrypted``) but none of the ``Decrypted*``
schema, and the cloud types this schema reuses are pinned here at ids that have
since moved in the API schema - ``photoSize#77bfb61b`` against the current API's
``photoSize#75c78e60``, for one. Reading a secret chat through Telethon's registry
would decode those against whatever the API schema says today. So the registry is
ours, keyed by the ids on this page, and Telethon's global ``tlobjects`` is never
mutated - the archived package's ``patch_tlobjects()`` was a process-wide side
effect this package does not need.

NAMING. A TL name is reused across layers with a different id each time
(``decryptedMessage`` exists three times). The LAST declaration on the page is the
current one and takes the clean class name; earlier ones are suffixed with their
id. Names are cosmetic - dispatch is by constructor id - so this only decides what
the package's own code types when it builds a message, and those are all current.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from typing import List, NamedTuple, Optional

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "telethon_secret_chat" / "schema" / "end-to-end.tl"
TARGET = ROOT / "telethon_secret_chat" / "schema" / "secret_tl.py"

# Historical names worth keeping readable. The layer-8 service wrapper is not a
# compatibility path we implement (Principle II puts layer 8 out of scope) but it
# is a shape we must still ACCEPT: protocol-reference.md §3.1 and §7.2 record a
# peer's NotifyLayer arriving inside it, and TDLib normalises it on receipt.
ALIASES = {"aa48327d": "DecryptedMessageService8"}


class Field(NamedTuple):
    name: str
    type: str
    flag_bit: Optional[int]  # None when unconditional
    flag_field: Optional[str]


class Ctor(NamedTuple):
    name: str
    id_hex: str
    fields: List[Field]
    result: str


def parse(text: str) -> List[Ctor]:
    out: List[Ctor] = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("//") or not line.endswith(";"):
            continue
        head, _, result = line[:-1].rpartition("=")
        head, result = head.strip(), result.strip()
        name_id, _, params = head.partition(" ")
        name, _, id_hex = name_id.partition("#")
        if not id_hex:
            continue
        fields = []
        for token in params.split():
            fname, _, ftype = token.partition(":")
            m = re.match(r"(\w+)\.(\d+)\?(.+)", ftype)
            if m:
                fields.append(Field(fname, m.group(3), int(m.group(2)), m.group(1)))
            else:
                fields.append(Field(fname, ftype, None, None))
        out.append(Ctor(name, id_hex.lower(), fields, result))
    return out


def class_names(ctors: List[Ctor]) -> dict:
    """Clean name to the newest declaration; the rest carry their id."""
    last = {}
    for i, c in enumerate(ctors):
        last[c.name] = i
    names = {}
    for i, c in enumerate(ctors):
        base = c.name[0].upper() + c.name[1:]
        if c.id_hex in ALIASES:
            names[c.id_hex] = ALIASES[c.id_hex]
        elif last[c.name] == i:
            names[c.id_hex] = base
        else:
            names[c.id_hex] = f"{base}_{c.id_hex}"
    return names


# --- per-type read/write fragments -------------------------------------------
# `w` appends to a list of byte strings; `r` is a BinaryReader.

WRITE = {
    "int": "w.append(_pack_int({v}))",
    "long": "w.append(_pack_long({v}))",
    "double": "w.append(_pack_double({v}))",
    "int128": "w.append(bytes({v}))",
    "int256": "w.append(bytes({v}))",
    "string": "w.append(_serialize_bytes({v}))",
    "bytes": "w.append(_serialize_bytes({v}))",
    "Bool": "w.append(_BOOL_TRUE if {v} else _BOOL_FALSE)",
}

READ = {
    "int": "r.read_int()",
    "long": "r.read_long()",
    "double": "r.read_double()",
    "int128": "r.read_large_int(bits=128)",
    "int256": "r.read_large_int(bits=256)",
    "string": "r.tgread_string()",
    "bytes": "r.tgread_bytes()",
    "Bool": "r.tgread_bool()",
}


def write_expr(ftype: str, value: str) -> List[str]:
    if ftype in WRITE:
        return [WRITE[ftype].format(v=value)]
    vec = re.match(r"[Vv]ector<(.+)>$", ftype)
    if vec:
        inner = vec.group(1)
        lines = [
            "w.append(_VECTOR)",
            f"w.append(_pack_int(len({value})))",
            f"for _item in {value}:",
        ]
        lines += ["    " + ln for ln in write_expr(inner, "_item")]
        return lines
    # Anything else is an object: it serializes itself.
    return [f"w.append(bytes({value}))"]


def read_expr(ftype: str) -> str:
    if ftype in READ:
        return READ[ftype]
    vec = re.match(r"[Vv]ector<(.+)>$", ftype)
    if vec:
        return f"_read_vector(r, lambda: {read_expr(vec.group(1))})"
    return "read_object(r)"


def emit(ctors: List[Ctor]) -> str:
    names = class_names(ctors)
    parts = [HEADER]
    for c in ctors:
        parts.append(emit_class(c, names[c.id_hex]))
    registry = "\n".join(f"    0x{c.id_hex.zfill(8)}: {names[c.id_hex]}," for c in ctors)
    parts.append(f"\n\nREGISTRY = {{\n{registry}\n}}\n")
    parts.append(
        "\n__all__ = [\n"
        + "".join(f'    "{names[c.id_hex]}",\n' for c in ctors)
        + '    "REGISTRY",\n    "SecretTLObject",\n    "read_object",\n'
        + '    "UnknownConstructor",\n]\n'
    )
    return "".join(parts)


def emit_class(c: Ctor, cls: str) -> str:
    flag_fields = [f.name for f in c.fields if f.type == "#"]
    real = [f for f in c.fields if f.type != "#"]
    args = ", ".join(f"{f.name}=None" for f in real)
    lines = [f"\n\nclass {cls}(SecretTLObject):"]
    lines.append(f'    """``{c.name}#{c.id_hex}`` -> ``{c.result}``."""')
    lines.append("")
    lines.append(f"    CONSTRUCTOR_ID = 0x{c.id_hex.zfill(8)}")
    lines.append(f'    TL_NAME = "{c.name}"')
    lines.append(f'    RESULT_TYPE = "{c.result}"')
    lines.append("")
    if real:
        lines.append(f"    def __init__(self, {args}):")
        for f in real:
            lines.append(f"        self.{f.name} = {f.name}")
    else:
        lines.append("    def __init__(self):")
        lines.append("        pass")
    lines.append("")

    # --- writer ---
    lines.append("    def _write_body(self, w):")
    body: List[str] = []
    for ff in flag_fields:
        bits = [
            f"(1 << {f.flag_bit} if {'self.' + f.name} "
            + ("else 0)" if f.type == "true" else "is not None else 0)")
            for f in c.fields
            if f.flag_field == ff
        ]
        expr = " | ".join(bits) if bits else "0"
        body.append(f"{ff} = {expr}")
        body.append(f"w.append(_pack_int({ff}))")
    for f in real:
        if f.type == "true":
            continue  # carried entirely by its flag bit
        if f.flag_bit is None:
            body += write_expr(f.type, f"self.{f.name}")
        else:
            body.append(f"if self.{f.name} is not None:")
            body += ["    " + ln for ln in write_expr(f.type, f"self.{f.name}")]
    if not body:
        body = ["return"]
    lines += ["        " + ln for ln in body]
    lines.append("")

    # --- reader ---
    lines.append("    @classmethod")
    lines.append("    def from_reader(cls, r):")
    rb: List[str] = []
    for ff in flag_fields:
        rb.append(f"{ff} = r.read_int()")
    for f in real:
        if f.flag_bit is None:
            rb.append(f"{f.name} = {read_expr(f.type)}")
        elif f.type == "true":
            rb.append(f"{f.name} = bool({f.flag_field} & (1 << {f.flag_bit}))")
        else:
            rb.append(
                f"{f.name} = {read_expr(f.type)} "
                f"if {f.flag_field} & (1 << {f.flag_bit}) else None"
            )
    call = ", ".join(f"{f.name}={f.name}" for f in real)
    rb.append(f"return cls({call})")
    lines += ["        " + ln for ln in rb]
    return "\n".join(lines) + "\n"


HEADER = '''# generated
# ruff: noqa
"""GENERATED from ``end-to-end.tl`` by ``tools/generate_schema.py``. Do not edit.

Telegram's end-to-end TL schema as Python. Every constructor id here is the one
published at https://core.telegram.org/schema/end-to-end; nothing in this file was
typed by hand, which is the point - protocol-reference.md quotes fifteen of these
ids and all fifteen are reproduced by the generator from the fetched page.

Exempt from the constitution's 800-line ceiling as generated code, and marked
``# generated`` on line 1 as the constitution requires.

Dispatch is by constructor id through ``REGISTRY``; Telethon's global
``alltlobjects`` registry is never read and never mutated.
"""

from __future__ import annotations

import struct

_VECTOR = b"\\x15\\xc4\\xb5\\x1c"
_BOOL_TRUE = b"\\xb5\\x75\\x72\\x99"
_BOOL_FALSE = b"\\x37\\x97\\x79\\xbc"


class UnknownConstructor(ValueError):
    """A constructor id this schema does not define.

    Raised rather than guessed. FR-011 wants an unknown action reported, not
    dropped, and a wrong guess at a shape is how a parser becomes an oracle.
    """

    def __init__(self, constructor_id: int):
        self.constructor_id = constructor_id
        super().__init__(f"unknown constructor 0x{constructor_id:08x}")


def _pack_int(value: int) -> bytes:
    return struct.pack("<i", value) if -(2**31) <= value < 2**31 else struct.pack("<I", value)


def _pack_long(value: int) -> bytes:
    return struct.pack("<q", value) if -(2**63) <= value < 2**63 else struct.pack("<Q", value)


def _pack_double(value: float) -> bytes:
    return struct.pack("<d", value)


def _serialize_bytes(data) -> bytes:
    """TL byte strings: a short length prefix, then padding to a multiple of four.

    The same rule Telethon's ``TLObject.serialize_bytes`` implements; written here
    so the generated module has no import from a Telethon private path.
    """
    if isinstance(data, str):
        data = data.encode("utf-8")
    elif not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("a TL string or bytes field takes str or bytes")
    data = bytes(data)
    if len(data) < 254:
        padding = (len(data) + 1) % 4
        if padding:
            padding = 4 - padding
        return bytes([len(data)]) + data + bytes(padding)
    padding = len(data) % 4
    if padding:
        padding = 4 - padding
    return b"\\xfe" + len(data).to_bytes(3, "little") + data + bytes(padding)


def _read_vector(r, read_item):
    marker = r.read_int(signed=False)
    if marker != 0x1CB5C415:
        raise ValueError("expected a vector")
    return [read_item() for _ in range(r.read_int())]


def read_object(r):
    """Read one boxed object using THIS schema's registry."""
    constructor_id = r.read_int(signed=False)
    cls = REGISTRY.get(constructor_id)
    if cls is None:
        raise UnknownConstructor(constructor_id)
    return cls.from_reader(r)


class SecretTLObject:
    """Base for every constructor below."""

    CONSTRUCTOR_ID = 0
    TL_NAME = ""
    RESULT_TYPE = ""

    def __bytes__(self) -> bytes:
        w = [_pack_int(self.CONSTRUCTOR_ID)]
        self._write_body(w)
        return b"".join(w)

    def _write_body(self, w):
        raise NotImplementedError

    @classmethod
    def from_reader(cls, r):
        raise NotImplementedError

    def to_dict(self) -> dict:
        return {"_": self.TL_NAME, **{k: v for k, v in vars(self).items()}}

    def __eq__(self, other) -> bool:
        return type(self) is type(other) and vars(self) == vars(other)

    def __repr__(self) -> str:
        """Names only, never values.

        Constitution Principle IV: these objects carry plaintext message bodies
        and, in the rekey actions, public exchange values. A default repr would
        print them the first time one reached a log line or a traceback - which is
        exactly how the archived package leaked plaintext at DEBUG (§8.8).
        """
        return f"{type(self).__name__}(<{len(vars(self))} fields>)"
'''


def main() -> int:
    ctors = parse(SCHEMA.read_text(encoding="utf-8"))
    if not ctors:
        print("no constructors parsed - is the schema file intact?", file=sys.stderr)
        return 1
    TARGET.write_text(emit(ctors), encoding="utf-8", newline="\n")
    # Formatted here rather than left to a human: CI runs `black --check .` over
    # everything, so a generated file that is not already black-clean turns
    # "regenerate the schema" into a two-step ritual someone will half-remember.
    subprocess.run([sys.executable, "-m", "black", "-q", str(TARGET)], check=True)
    print(f"generated {TARGET.relative_to(ROOT)}: {len(ctors)} constructors")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
