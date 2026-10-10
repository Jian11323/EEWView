# -*- coding: utf-8 -*-
"""Safe UTF-8 file helpers. Agents must use this (or pathlib write_text utf-8), never PowerShell redirection."""
from __future__ import annotations

from pathlib import Path


def read_text_healed(path: Path) -> str:
    raw = path.read_bytes()
    # collapse broken middle-dot / multiply sequences first
    raw = raw.replace(b"\xc2\xc2\xb7", "·".encode("utf-8"))
    raw = raw.replace(b"\xc3\x82\xc2\xb7", "·".encode("utf-8"))  # UTF-8 of mojibake Â·
    raw = raw.replace(b"\xb7", "·".encode("utf-8"))
    out = bytearray()
    i = 0
    while i < len(raw):
        b = raw[i]
        if b < 0x80:
            out.append(b)
            i += 1
            continue
        if 0xC2 <= b <= 0xDF and i + 1 < len(raw) and 0x80 <= raw[i + 1] <= 0xBF:
            out.extend(raw[i : i + 2])
            i += 2
            continue
        if (
            0xE0 <= b <= 0xEF
            and i + 2 < len(raw)
            and 0x80 <= raw[i + 1] <= 0xBF
            and 0x80 <= raw[i + 2] <= 0xBF
        ):
            out.extend(raw[i : i + 3])
            i += 3
            continue
        if b == 0xD7:
            out.extend("×".encode("utf-8"))
            i += 1
            continue
        i += 1
    return out.decode("utf-8")


def write_text_utf8(path: Path, text: str) -> None:
    path.write_bytes(text.encode("utf-8"))
