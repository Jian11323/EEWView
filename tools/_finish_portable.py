# -*- coding: utf-8 -*-
"""Write usage note and zip dist/EEWView as Beta portable."""
import re
import sys
import zipfile
from pathlib import Path

out = Path(sys.argv[1])
root = Path(sys.argv[2])
cargo = (root / "Cargo.toml").read_text(encoding="utf-8")
m = re.search(r'(?m)^version\s*=\s*"([^"]+)"', cargo)
ver = m.group(1) if m else "0.0.0"

usage = f"""EEWView 便携测试包
================

版本：Beta {ver}
双击 eewview.exe 即可运行（勿拆散 assets / geodata / config 相对位置）。

【首次使用】
1. 打开后点「设置」→ API与数据源，用 lk_ 换票绑定 Jian（可选）。
2. 未配置 Jian 时，Wolfx / P2P / Pancakes 仍可用；Jian 健康态为「异常」。
3. 用户配置写到本机 AppData，不回写本目录。

【版本规则】
- PATCH（x.y.Z）：修 bug / 微调，无新能力
- MINOR（x.Y.0）：新功能或行为变更
- MAJOR（X.0.0）：不兼容变更，或结束 Beta 转正式版
"""
(out / "使用说明.txt").write_bytes(usage.encode("utf-8-sig"))

zip_path = root / "dist" / f"EEWView_Beta_{ver}_portable.zip"
if zip_path.exists():
    zip_path.unlink()
skip_names = {".DS_Store", "Thumbs.db"}
count = 0
with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
    for path in out.rglob("*"):
        if not path.is_file() or path.name in skip_names:
            continue
        arc = Path("EEWView") / path.relative_to(out)
        zf.write(path, arc.as_posix())
        count += 1
mb = zip_path.stat().st_size / (1024 * 1024)
print(f"usage+zip OK Beta {ver} files={count} size={mb:.1f} MB")
print(zip_path)
