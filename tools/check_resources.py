# -*- coding: utf-8 -*-
"""资源完整性检查：media/ + icon/ + geodata 关键点与 catalog 事件文件。

用法（仓库根）:
  python tools/check_resources.py
退出码 0=通过，1=失败。
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

REQUIRED_DIRS = (
    "media/sound",
    "media/travel",
    "media/palette",
    "media/place",
    "media/stations",
    "media/fonts/lxgw-wenkai-v1.522",
    "icon/intensity",
    "icon/markers",
    "icon/brand",
    "geodata",
    "config",
)

REQUIRED_FILES = (
    "media/sound/catalog.json",
    "media/place/fe_fix.txt",
    "media/stations/pixels.csv",
    "media/stations/nied_point.csv",
    "media/palette/cn_intensity.toml",
    "media/palette/jma_shindo.toml",
    "media/fonts/lxgw-wenkai-v1.522/LXGWWenKai-Regular.ttf",
    "icon/brand/app.ico",
    "icon/brand/app.png",
    "icon/brand/logo.svg",
    "icon/brand/splash_bg.png",
    "icon/markers/cross.svg",
    "icon/markers/red.svg",
    "icon/markers/green.svg",
    "config/default.toml",
)

# 发行/运行不必带齐的包（如 countdown 可选、srev 需在仓内）
OPTIONAL_PACKS = frozenset({"countdown"})


def fail(msg: str, errors: list[str]) -> None:
    errors.append(msg)


def check_layout(errors: list[str]) -> None:
    if (ROOT / "assets").exists():
        fail("遗留目录 assets/ 仍存在，应已迁移到 media/ 与 icon/", errors)
    for rel in REQUIRED_DIRS:
        p = ROOT / rel
        if not p.is_dir():
            fail(f"缺少目录: {rel}", errors)
    for rel in REQUIRED_FILES:
        p = ROOT / rel
        if not p.is_file():
            fail(f"缺少文件: {rel}", errors)


def check_catalog(errors: list[str]) -> None:
    cat_path = ROOT / "media" / "sound" / "catalog.json"
    if not cat_path.is_file():
        return
    try:
        cat = json.loads(cat_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as e:
        fail(f"catalog.json 无法解析: {e}", errors)
        return
    sound_root = ROOT / "media" / "sound"
    packs = cat.get("packs") or {}
    for pack_name, pack in packs.items():
        events = (pack or {}).get("events") or {}
        for ev, rel in events.items():
            path = sound_root / rel
            if pack_name in OPTIONAL_PACKS and not path.is_file():
                continue
            if not path.is_file():
                fail(f"catalog 缺文件: packs.{pack_name}.events.{ev} -> {rel}", errors)


def check_svgs(errors: list[str]) -> None:
    icon = ROOT / "icon"
    if not icon.is_dir():
        return
    svgs = list(icon.rglob("*.svg"))
    if len(svgs) < 10:
        fail(f"icon/ 下 SVG 过少 ({len(svgs)})", errors)
    for p in svgs:
        try:
            data = p.read_bytes()
        except OSError as e:
            fail(f"无法读取 {p.relative_to(ROOT)}: {e}", errors)
            continue
        if len(data) < 20:
            fail(f"SVG 过小: {p.relative_to(ROOT)}", errors)
        # 粗检：应为 XML/SVG 文本
        head = data[:64].lstrip()
        if not (head.startswith(b"<") or head.startswith(b"\xef\xbb\xbf<")):
            fail(f"SVG 头异常: {p.relative_to(ROOT)}", errors)


def check_geodata(errors: list[str]) -> None:
    geo = ROOT / "geodata"
    if not geo.is_dir():
        return
    # 暗色底图优先目录或要石回退至少其一
    gq = geo / "globalquake"
    ka = geo / "kanameishi"
    if not gq.is_dir() and not ka.is_dir():
        fail("geodata 缺少 globalquake/ 与 kanameishi/", errors)


def main() -> int:
    errors: list[str] = []
    check_layout(errors)
    check_catalog(errors)
    check_svgs(errors)
    check_geodata(errors)
    if errors:
        print(f"check_resources: FAIL ({len(errors)})")
        for e in errors:
            print(f"  - {e}")
        return 1
    print("check_resources: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
