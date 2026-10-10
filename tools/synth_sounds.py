#!/usr/bin/env python3
"""Generate minimal CC0 WAV cues for eew / station / ui packs."""

from __future__ import annotations

import math
import struct
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "assets" / "sound"
RATE = 22050

SPECS = {
    "ui/connect_ok.wav": (880, 0.12, 0.25),
    "ui/connect_fail.wav": (220, 0.18, 0.3),
    "ui/notify.wav": (660, 0.08, 0.2),
    "eew/issue.wav": (520, 0.2, 0.35),
    "eew/update.wav": (480, 0.12, 0.25),
    "eew/final.wav": (400, 0.25, 0.4),
    "eew/cancel.wav": (300, 0.2, 0.35),
    "eew/prompt.wav": (600, 0.1, 0.2),
    "eew/arrive_p.wav": (700, 0.08, 0.15),
    "eew/arrive_s.wav": (350, 0.1, 0.2),
    "station/weak.wav": (440, 0.06, 0.12),
    "station/mid.wav": (520, 0.08, 0.15),
    "station/strong.wav": (300, 0.12, 0.22),
}


def write_tone(path: Path, freq: float, dur: float, vol: float) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    n = int(RATE * dur)
    with wave.open(str(path), "w") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        frames = bytearray()
        for i in range(n):
            t = i / RATE
            env = min(1.0, i / 200.0) * min(1.0, (n - i) / 400.0)
            sample = int(32767 * vol * env * math.sin(2 * math.pi * freq * t))
            frames += struct.pack("<h", sample)
        w.writeframes(frames)
    print("wrote", path)


def main() -> None:
    for rel, args in SPECS.items():
        write_tone(ROOT / rel, *args)
    print("done")


if __name__ == "__main__":
    main()
