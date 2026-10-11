# -*- coding: utf-8 -*-
"""Windows 便携包：release 构建 + 组装 + zip（纯 Python，避免 PS 卡死）。"""
from __future__ import annotations

import re
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def log(msg: str) -> None:
    print(msg, flush=True)


def rm_tree(path: Path) -> None:
    if not path.exists():
        return
    for attempt in range(8):
        try:
            shutil.rmtree(path)
            return
        except OSError as e:
            log(f"remove retry {attempt + 1}: {e}")
            time.sleep(0.6)
    # last resort: rename aside
    bak = path.with_name(path.name + f".old_{int(time.time())}")
    try:
        path.rename(bak)
        log(f"renamed locked dir -> {bak}")
    except OSError as e:
        raise SystemExit(f"无法清理输出目录 {path}: {e}") from e


def copy_tree(src: Path, dst: Path) -> None:
    log(f"copy {src.name} ...")
    shutil.copytree(
        src,
        dst,
        ignore=shutil.ignore_patterns(
            "__pycache__",
            "*.pyc",
            ".git",
            "local.toml",
            "Thumbs.db",
            ".DS_Store",
        ),
        dirs_exist_ok=False,
    )


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
    return m.group(1) if m else "0.0.0"


def write_usage(out: Path, ver: str) -> None:
    usage = f"""EEW View 便携测试包
================

版本：Beta {ver}
双击 jian.exe 即可运行（勿拆散 assets / geodata / config 相对位置）。

【首次使用】
1. 打开后点「设置」→ API与数据源，用 lk_ 换票绑定 Jian（可选）。
2. 未配置 Jian 时，Wolfx / P2P / Pancakes 仍可用；Jian 健·态为「异常」。
3. 用·配置写到本机 AppData，不回写本目录。
4. 运行日志在本目录 logs/（按日滚动）；设置 → 高级可打开。

【版本规则】
- PATCH（x.y.Z）：修 bug / 微调，无新能力
- MINOR（x.Y.0）：新功能或行为变更
- MAJOR（X.0.0）：不兼容变更，或结束 Beta 转正式版
"""
    (out / "使用说明.txt").write_bytes(usage.encode("utf-8-sig"))


def zip_out(out: Path, ver: str) -> Path:
    zip_path = ROOT / "dist" / f"EEWView_Beta_{ver}_portable.zip"
    if zip_path.exists():
        zip_path.unlink()
    skip = {".DS_Store", "Thumbs.db"}
    n = 0
    log(f"zip -> {zip_path.name} ...")
    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=5) as zf:
        for path in out.rglob("*"):
            if not path.is_file() or path.name in skip:
                continue
            # ·过 travel/raw（若误·入）
            parts = set(path.parts)
            if "raw" in parts and "travel" in parts:
                continue
            arc = Path("EEWView") / path.relative_to(out)
            zf.write(path, arc.as_posix())
            n += 1
            if n % 100 == 0:
                log(f"  zipped {n} files...")
    mb = zip_path.stat().st_size / (1024 * 1024)
    log(f"zip OK files={n} size={mb:.1f} MB")
    return zip_path


def main() -> int:
    ver = workspace_version()
    log(f"EEW View Beta {ver} portable pack")

    skip_build = "--skip-build" in sys.argv
    exe_src = ROOT / "target" / "release" / "jian.exe"
    if not skip_build or not exe_src.is_file():
        log("cargo build -p jian-app --release")
        r = subprocess.run(
            ["cargo", "build", "-p", "jian-app", "--release"],
            cwd=ROOT,
        )
        if r.returncode != 0:
            return r.returncode
    else:
        log("skip build, use existing release exe")

    if not exe_src.is_file():
        log(f"missing {exe_src}")
        return 1

    out = ROOT / "dist" / "EEWView"
    log(f"clean {out}")
    rm_tree(out)
    out.mkdir(parents=True, exist_ok=True)

    shutil.copy2(exe_src, out / "jian.exe")
    for name in ("README.md", "LICENSE"):
        src = ROOT / name
        if src.is_file():
            shutil.copy2(src, out / name)

    copy_tree(ROOT / "config", out / "config")
    local = out / "config" / "local.toml"
    if local.is_file():
        local.unlink()

    copy_tree(ROOT / "assets", out / "assets")
    raw = out / "assets" / "travel" / "raw"
    if raw.is_dir():
        shutil.rmtree(raw)

    copy_tree(ROOT / "geodata", out / "geodata")

    write_usage(out, ver)
    zpath = zip_out(out, ver)
    log(f"DIR  {out}")
    log(f"ZIP  {zpath}")
    log(f"RUN  {out / 'jian.exe'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
