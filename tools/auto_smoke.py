# -*- coding: utf-8 -*-
"""StarQuake 自动化巡检编排：cargo test → 编译 → STARQUAKE_AUTOTEST 启动 → 读报告。"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(r"D:\我的文档\桌面\脚本程序\starquake")
REPORT = ROOT / "logs" / "autotest_report.json"
EXE_DEBUG = ROOT / "target" / "debug" / "starquake.exe"
EXE_RELEASE = ROOT / "target" / "release" / "starquake.exe"


def run(cmd: list[str], cwd: Path | None = None, env: dict | None = None, timeout: int = 600) -> int:
    print(f"\n>>> {' '.join(cmd)}", flush=True)
    e = os.environ.copy()
    if env:
        e.update(env)
    p = subprocess.run(cmd, cwd=str(cwd or ROOT), env=e, timeout=timeout)
    return p.returncode


def main() -> int:
    if REPORT.exists():
        REPORT.unlink()

    print("=== 1/4 cargo test --workspace ===", flush=True)
    rc = run(["cargo", "test", "--workspace", "--", "--test-threads=4"], timeout=900)
    if rc != 0:
        print("FAIL: cargo test", flush=True)
        return rc

    print("=== 2/4 cargo build -p starquake-app ===", flush=True)
    rc = run(["cargo", "build", "-p", "starquake-app"], timeout=900)
    if rc != 0:
        print("FAIL: cargo build", flush=True)
        return rc

    exe = EXE_DEBUG if EXE_DEBUG.is_file() else EXE_RELEASE
    if not exe.is_file():
        print("FAIL: no starquake.exe", flush=True)
        return 2

    print(f"=== 3/4 launch AUTOTEST {exe} ===", flush=True)
    env = {
        "STARQUAKE_AUTOTEST": "1",
        "STARQUAKE_ROOT": str(ROOT),
        "RUST_LOG": "info,starquake=info",
    }
    proc = subprocess.Popen([str(exe)], cwd=str(ROOT), env={**os.environ, **env})
    print(f"pid={proc.pid}", flush=True)

    deadline = time.time() + 100
    while time.time() < deadline:
        if REPORT.is_file():
            # 等进程退出
            try:
                proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                proc.kill()
            break
        if proc.poll() is not None:
            print(f"process exited early code={proc.returncode}", flush=True)
            break
        time.sleep(0.5)
    else:
        print("TIMEOUT waiting report; killing", flush=True)
        proc.kill()
        return 3

    print("=== 4/4 report ===", flush=True)
    if not REPORT.is_file():
        print("FAIL: no autotest_report.json", flush=True)
        return 4
    data = json.loads(REPORT.read_text(encoding="utf-8"))
    print(json.dumps(data, ensure_ascii=False, indent=2), flush=True)
    ok = bool(data.get("ok"))
    failed = [c for c in data.get("checks", []) if not c.get("pass")]
    if failed:
        print("\nFAILED CHECKS:", flush=True)
        for c in failed:
            print(f"  - {c.get('name')}: {c.get('detail')}", flush=True)
    print(f"\nSUMMARY ok={ok} passed={data.get('passed')} failed={data.get('failed')} elapsed_ms={data.get('elapsed_ms')}", flush=True)
    return 0 if ok else 5


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:
        print(f"ORCHESTRATOR ERROR: {e}", flush=True)
        sys.exit(9)
