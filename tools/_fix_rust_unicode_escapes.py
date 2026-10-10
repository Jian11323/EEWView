# -*- coding: utf-8 -*-
"""Fix JS-style \\uXXXX escapes to Rust \\u{XXXX} in settings sources."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = [
    ROOT / "crates/jian-ui/src/settings.rs",
    ROOT / "crates/jian-ui/src/settings/filters_ui.rs",
]


def fix_text(text: str) -> tuple[str, int]:
    # Convert \uXXXX -> \u{XXXX}; skip already-braced forms.
    pattern = re.compile(r"(?<!\{)\\u([0-9a-fA-F]{4})(?!\})")
    new, n = pattern.subn(r"\\u{\1}", text)
    # Collapse accidental double braces from re-runs.
    new2, n2 = re.subn(r"\\u\{\{([0-9a-fA-F]{4})\}\}", r"\\u{\1}", new)
    return new2, n + n2


def main() -> None:
    for path in FILES:
        raw = path.read_text(encoding="utf-8")
        fixed, n = fix_text(raw)
        path.write_text(fixed, encoding="utf-8", newline="\n")
        samples = re.findall(r"\\u\{[0-9a-fA-F]+\}", fixed)[:6]
        print(f"{path.relative_to(ROOT)}: {n} replacements; samples={samples}")


if __name__ == "__main__":
    main()
