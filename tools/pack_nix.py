# -*- coding: utf-8 -*-
"""写出 NixOS/flake 安装说明到 dist/（本机有 nix 时再 nix build）。"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def log(msg: str) -> None:
    print(msg, flush=True)


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else "0.0.0"


def main() -> int:
    ver = workspace_version()
    if not (ROOT / "flake.nix").is_file():
        log("missing flake.nix")
        return 1
    out = ROOT / "dist"
    out.mkdir(parents=True, exist_ok=True)
    note = out / f"StarQuake_{ver}_nixos.txt"
    body = f"""StarQuake NixOS / flake 安装（{ver}）
========================================

一、源码构建（推荐，仓库根 flake.nix）
  nix run .#
  nix profile install .#
  nix build .#   # 结果在 result/

二、预构建便携 tar 包装（无需编译）
  1) 取得 StarQuake_{ver}_linux_x86_64.tar.gz（CI 或 pack_linux.py）
  2) 复制到 packaging/nixos/
  3) nix-env -if packaging/nixos/default.nix

入口命令：starquake（STARQUAKE_ROOT 指向 share/starquake）。
本机 Windows 请在 NixOS / WSL2+Nix / CI 上执行 nix 命令。
"""
    note.write_bytes(body.encode("utf-8"))
    log(f"NOTE {note}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
