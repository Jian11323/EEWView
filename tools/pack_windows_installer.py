# -*- coding: utf-8 -*-
"""Windows 安装包：基于已组装的 dist/EEWView + NSIS → setup.exe。"""
from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NSIS_URL = "https://sourceforge.net/projects/nsis/files/NSIS%203/3.10/nsis-3.10.zip/download"
NSIS_CACHE = ROOT / "target" / "nsis-3.10"


def log(msg: str) -> None:
    print(msg, flush=True)


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else "0.0.0"


def find_makensis() -> Path | None:
    which = shutil.which("makensis")
    if which:
        return Path(which)
    for c in (
        NSIS_CACHE / "makensis.exe",
        Path(r"C:\Program Files (x86)\NSIS\makensis.exe"),
        Path(r"C:\Program Files\NSIS\makensis.exe"),
    ):
        if c.is_file():
            return c
    return None


def ensure_nsis() -> Path:
    found = find_makensis()
    if found:
        return found
    NSIS_CACHE.parent.mkdir(parents=True, exist_ok=True)
    zip_path = ROOT / "target" / "nsis-3.10.zip"
    if not zip_path.is_file():
        log(f"download NSIS -> {zip_path.name}")
        # SourceForge / GitHub may need proxy from env
        req = urllib.request.Request(
            NSIS_URL,
            headers={"User-Agent": "EEWView-packager"},
        )
        with urllib.request.urlopen(req, timeout=180) as resp, zip_path.open("wb") as f:
            shutil.copyfileobj(resp, f)
    log("extract NSIS ...")
    if NSIS_CACHE.exists():
        shutil.rmtree(NSIS_CACHE)
    with zipfile.ZipFile(zip_path) as zf:
        zf.extractall(ROOT / "target")
    # archive root is nsis-3.10/
    extracted = ROOT / "target" / "nsis-3.10"
    if not extracted.is_dir():
        # some mirrors nest differently
        for p in (ROOT / "target").iterdir():
            if p.is_dir() and (p / "makensis.exe").is_file():
                extracted = p
                break
    if extracted.resolve() != NSIS_CACHE.resolve():
        if NSIS_CACHE.exists():
            shutil.rmtree(NSIS_CACHE)
        extracted.rename(NSIS_CACHE)
    exe = NSIS_CACHE / "makensis.exe"
    if not exe.is_file():
        raise SystemExit(f"makensis not found after extract: {NSIS_CACHE}")
    return exe


def ensure_portable() -> Path:
    dist = ROOT / "dist" / "EEWView"
    exe = dist / "eewview.exe"
    if exe.is_file() and (dist / "assets").is_dir():
        return dist
    log("portable dir missing → run pack_windows.py first")
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "pack_windows.py")], cwd=ROOT)
    if r.returncode != 0:
        raise SystemExit(r.returncode)
    if not exe.is_file():
        raise SystemExit(f"missing {exe}")
    return dist


def main() -> int:
    ver = workspace_version()
    log(f"EEW View Beta {ver} Windows installer")
    dist = ensure_portable()
    makensis = ensure_nsis()
    out = ROOT / "dist" / f"EEWView_Beta_{ver}_setup.exe"
    if out.exists():
        out.unlink()

    # ASCII staging paths — avoid NSIS/cmdline Unicode path issues
    stage_root = Path(os.environ.get("TEMP", tempfile.gettempdir())) / "eewview_nsis"
    stage_dist = stage_root / "EEWView"
    stage_out = stage_root / f"EEWView_Beta_{ver}_setup.exe"
    if stage_root.exists():
        shutil.rmtree(stage_root, ignore_errors=True)
    stage_root.mkdir(parents=True, exist_ok=True)
    log(f"stage copy -> {stage_dist}")
    shutil.copytree(dist, stage_dist)

    nsi = ROOT / "tools" / "pack_windows_installer.nsi"
    cmd = [
        str(makensis),
        f"/DVERSION={ver}",
        f"/DDIST_DIR={stage_dist}",
        f"/DOUT_FILE={stage_out}",
        str(nsi),
    ]
    log(" ".join(cmd))
    r = subprocess.run(cmd, cwd=str(ROOT / "tools"))
    if r.returncode != 0:
        return r.returncode
    if not stage_out.is_file():
        raise SystemExit(f"installer not produced: {stage_out}")
    shutil.copy2(stage_out, out)
    mb = out.stat().st_size / (1024 * 1024)
    log(f"SETUP {out} ({mb:.1f} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
