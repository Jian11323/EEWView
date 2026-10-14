# -*- coding: utf-8 -*-
"""便携/安装打包共用：版本读取与资源校验。"""
from __future__ import annotations

import re
from pathlib import Path

# 发行包必须带上的品牌资源（启动界面图编译嵌入，仍随包分发便于署名与核对）
REQUIRED_BRAND = (
    "app.ico",
    "app.png",
    "logo.svg",
    "splash_bg.png",
)


def workspace_version(root: Path) -> str:
    text = (root / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else "0.0.0"


def assert_brand_assets(root: Path) -> None:
    brand = root / "icon" / "brand"
    missing = [n for n in REQUIRED_BRAND if not (brand / n).is_file()]
    if missing:
        raise SystemExit(
            "缺少品牌资源，无法打包: "
            + ", ".join(f"icon/brand/{n}" for n in missing)
        )


def copy_ignore_patterns() -> tuple[str, ...]:
    return (
        "__pycache__",
        "*.pyc",
        ".git",
        "local.toml",
        "Thumbs.db",
        ".DS_Store",
        # 仅嵌入 Regular；其它字重体积大且未使用
        "LXGWWenKai-Light.ttf",
        "LXGWWenKai-Medium.ttf",
        "LXGWWenKaiMono-*.ttf",
    )
