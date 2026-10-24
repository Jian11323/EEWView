# -*- coding: utf-8 -*-
"""StarQuake 全项目 QA 编排。

阶段：
  1) cargo test --workspace
  2) cargo build -p starquake-app --release
  3) STARQUAKE_AUTOTEST 进程内巡检
  4) 测站适配/稳定性 verify_station_adapters.py
  5) 全数据源压测 stress_live_sources.py（FULL）

用法（仓库根）:
  python tools/full_qa.py
  set EEWVIEW_STRESS_SECS=180&& set EEWVIEW_STATION_SECS=60&& python tools/full_qa.py

环境变量：
  EEWVIEW_STRESS_SECS     压测秒数，默认 180
  EEWVIEW_STATION_SECS    测站验证秒数，默认 60
  EEWVIEW_QA_SKIP_STRESS  1=跳过压测
  EEWVIEW_QA_SKIP_STATION 1=跳过测站验证
  EEWVIEW_QA_SKIP_TEST    1=跳过 cargo test
报告：logs/full_qa_YYYYMMDD_HHMMSS.json
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AUTOTEST_REPORT = ROOT / "logs" / "autotest_report.json"
EXE = ROOT / "target" / "release" / "starquake.exe"


def _truthy(v: str | None) -> bool:
    return (v or "").strip().lower() in ("1", "true", "yes", "on")


def run(cmd: list[str], *, env: dict | None = None, timeout: int = 1200) -> dict:
    print(f"\n>>> {' '.join(cmd)}", flush=True)
    e = os.environ.copy()
    if env:
        e.update(env)
    t0 = time.perf_counter()
    try:
        # 长任务直接刷屏，便于盯进度；报告只记退出码与耗时
        p = subprocess.run(cmd, cwd=str(ROOT), env=e, timeout=timeout)
        out = {
            "cmd": cmd,
            "ok": p.returncode == 0,
            "code": p.returncode,
            "secs": round(time.perf_counter() - t0, 1),
        }
    except subprocess.TimeoutExpired:
        out = {
            "cmd": cmd,
            "ok": False,
            "code": -1,
            "secs": round(time.perf_counter() - t0, 1),
            "error": f"timeout {timeout}s",
        }
    status = "PASS" if out["ok"] else "FAIL"
    print(f"{status} ({out['secs']}s) code={out.get('code')}", flush=True)
    return out


def run_autotest() -> dict:
    if AUTOTEST_REPORT.exists():
        AUTOTEST_REPORT.unlink()
    if not EXE.is_file():
        return {"ok": False, "error": f"missing {EXE}"}
    env = {
        "STARQUAKE_AUTOTEST": "1",
        "STARQUAKE_ROOT": str(ROOT),
        "RUST_LOG": "info,starquake=info",
    }
    print(f"\n>>> AUTOTEST {EXE}", flush=True)
    t0 = time.perf_counter()
    proc = subprocess.Popen(
        [str(EXE)],
        cwd=str(ROOT),
        env={**os.environ, **env},
    )
    deadline = time.time() + 100
    while time.time() < deadline:
        if AUTOTEST_REPORT.is_file() and AUTOTEST_REPORT.stat().st_size > 50:
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
            break
        if proc.poll() is not None:
            break
        time.sleep(0.4)
    else:
        proc.kill()
        return {"ok": False, "error": "autotest timeout", "secs": round(time.perf_counter() - t0, 1)}

    if not AUTOTEST_REPORT.is_file():
        return {"ok": False, "error": "no autotest_report.json", "secs": round(time.perf_counter() - t0, 1)}
    data = json.loads(AUTOTEST_REPORT.read_text(encoding="utf-8"))
    return {
        "ok": bool(data.get("ok")),
        "secs": round(time.perf_counter() - t0, 1),
        "passed": data.get("passed"),
        "failed": data.get("failed"),
        "elapsed_ms": data.get("elapsed_ms"),
        "checks": data.get("checks"),
    }


def latest_log(prefix: str) -> Path | None:
    logs = ROOT / "logs"
    if not logs.is_dir():
        return None
    files = sorted(logs.glob(f"{prefix}_*.json"), key=lambda p: p.stat().st_mtime, reverse=True)
    return files[0] if files else None


def main() -> int:
    stress_secs = os.environ.get("EEWVIEW_STRESS_SECS", "180")
    station_secs = os.environ.get("EEWVIEW_STATION_SECS", "60")
    skip_test = _truthy(os.environ.get("EEWVIEW_QA_SKIP_TEST"))
    skip_station = _truthy(os.environ.get("EEWVIEW_QA_SKIP_STATION"))
    skip_stress = _truthy(os.environ.get("EEWVIEW_QA_SKIP_STRESS"))

    est = 0
    if not skip_test:
        est += 240
    est += 90  # build
    est += 30  # autotest
    if not skip_station:
        # 三路测站串行
        est += int(float(station_secs)) * 3 + 30
    if not skip_stress:
        est += int(float(stress_secs)) + 30
    print(
        f"StarQuake full QA 开始。预估约 {est // 60} 分 {est % 60} 秒 "
        f"(test={'skip' if skip_test else 'on'}, station={station_secs}s×3, stress={stress_secs}s FULL)",
        flush=True,
    )

    started = datetime.now(timezone.utc).isoformat()
    stages: dict[str, dict] = {}
    hard_fail = False

    if not skip_test:
        stages["cargo_test"] = run(
            ["cargo", "test", "--workspace", "--", "--test-threads=4"],
            timeout=1200,
        )
        if not stages["cargo_test"]["ok"]:
            hard_fail = True
    else:
        stages["cargo_test"] = {"ok": True, "skipped": True}

    stages["cargo_build_release"] = run(
        ["cargo", "build", "-p", "starquake-app", "--release"],
        timeout=900,
    )
    if not stages["cargo_build_release"]["ok"]:
        hard_fail = True

    if stages["cargo_build_release"]["ok"]:
        stages["autotest"] = run_autotest()
        print(
            f"AUTOTEST ok={stages['autotest'].get('ok')} "
            f"passed={stages['autotest'].get('passed')} failed={stages['autotest'].get('failed')}",
            flush=True,
        )
        if not stages["autotest"]["ok"]:
            hard_fail = True
    else:
        stages["autotest"] = {"ok": False, "skipped": True, "error": "build failed"}

    if not skip_station and not hard_fail:
        before = latest_log("station_verify")
        # 三路测站串行，总墙钟约 3×secs + 余量
        stages["station_verify"] = run(
            [sys.executable, str(ROOT / "tools" / "verify_station_adapters.py")],
            env={"EEWVIEW_STATION_SECS": str(station_secs)},
            timeout=int(float(station_secs)) * 3 + 180,
        )
        after = latest_log("station_verify")
        if after and after != before:
            stages["station_verify"]["report"] = str(after.relative_to(ROOT))
        if not stages["station_verify"]["ok"]:
            hard_fail = True
    else:
        stages["station_verify"] = {"ok": True, "skipped": True}

    if not skip_stress:
        before = latest_log("stress_live")
        stages["stress_live"] = run(
            [sys.executable, str(ROOT / "tools" / "stress_live_sources.py")],
            env={
                "EEWVIEW_STRESS": "1",
                "EEWVIEW_STRESS_FULL": "1",
                "EEWVIEW_STRESS_SECS": str(stress_secs),
                "EEWVIEW_STRESS_WS": "1",
            },
            timeout=int(float(stress_secs)) + 180,
        )
        after = latest_log("stress_live")
        if after and after != before:
            stages["stress_live"]["report"] = str(after.relative_to(ROOT))
            try:
                sdata = json.loads(after.read_text(encoding="utf-8"))
                stages["stress_live"]["summary"] = sdata.get("summary")
                stages["stress_live"]["jian_token_present"] = sdata.get("jian_token_present")
            except Exception:
                pass
        if not stages["stress_live"]["ok"]:
            hard_fail = True
    else:
        stages["stress_live"] = {"ok": True, "skipped": True}

    logs = ROOT / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    out_path = logs / f"full_qa_{stamp}.json"
    report = {
        "started_utc": started,
        "finished_utc": datetime.now(timezone.utc).isoformat(),
        "ok": not hard_fail,
        "stages": stages,
    }
    out_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    print("\n======== FULL QA SUMMARY ========", flush=True)
    for name, st in stages.items():
        mark = "SKIP" if st.get("skipped") else ("PASS" if st.get("ok") else "FAIL")
        extra = ""
        if name == "autotest" and st.get("passed") is not None:
            extra = f" checks={st.get('passed')}/{int(st.get('passed') or 0)+int(st.get('failed') or 0)}"
        if st.get("report"):
            extra += f" report={st['report']}"
        print(f"  {mark:4} {name}{extra}", flush=True)
    print(f"overall={'PASS' if not hard_fail else 'FAIL'} -> {out_path.relative_to(ROOT)}", flush=True)
    return 0 if not hard_fail else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("interrupted", flush=True)
        raise SystemExit(130)
