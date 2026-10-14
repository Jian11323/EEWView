# -*- coding: utf-8 -*-
"""核对三条测站中转：适配字段形状 + 长连接稳定性（无需鉴权）。

用法（仓库根）:
  python tools/verify_station_adapters.py
  set EEWVIEW_STATION_SECS=90 && python tools/verify_station_adapters.py

报告：logs/station_verify_YYYYMMDD_HHMMSS.json（勿提交）
退出码：适配/稳定性不合格为 1。
"""
from __future__ import annotations

import asyncio
import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]

ENDPOINTS = {
    "kmoni": {
        "url": "wss://api.sismotide.top/kmoni",
        "expect_type": "kmoni",
        "data_key": "int",
    },
    "s-net": {
        "url": "wss://api.sismotide.top/s-net",
        "expect_type": "s-net",
        "data_key": "int",
        # 约每秒：time=资料时刻，timestamp=对时时钟
        "need_time_fields": True,
    },
    "kma-station": {
        "url": "wss://api.sismotide.top/kma-station",
        "expect_type": "kma-station",
        "data_key": "mmi",
    },
}


def _secs() -> float:
    return float(os.environ.get("EEWVIEW_STATION_SECS", "90"))


async def verify_one(name: str, spec: dict[str, Any], duration_s: float) -> dict[str, Any]:
    import websockets

    url = spec["url"]
    expect_type = spec["expect_type"]
    data_key = spec["data_key"]
    need_time = bool(spec.get("need_time_fields"))

    out: dict[str, Any] = {
        "name": name,
        "url": url,
        "ok": False,
        "reconnects": 0,
        "frames": 0,
        "meta_ok": False,
        "meta_n": 0,
        "data_frames": 0,
        "data_len_match": 0,
        "data_len_mismatch": 0,
        "time_field_ok": 0,
        "timestamp_ok": 0,
        "timestamp_monotonic": True,
        "max_gap_s": 0.0,
        "avg_gap_s": 0.0,
        "errors": [],
        "adapter_notes": [],
    }

    last_ts: int | None = None
    last_recv: float | None = None
    gaps: list[float] = []
    meta_n = 0
    deadline = time.monotonic() + duration_s
    backoff = 1.0

    while time.monotonic() < deadline:
        try:
            async with websockets.connect(
                url,
                open_timeout=15,
                ping_interval=20,
                ping_timeout=20,
                max_size=8 * 1024 * 1024,
            ) as ws:
                backoff = 1.0
                while time.monotonic() < deadline:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        break
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=min(20.0, remaining))
                    except asyncio.TimeoutError:
                        out["errors"].append("recv timeout >20s")
                        break
                    if isinstance(raw, (bytes, bytearray)):
                        continue
                    now = time.monotonic()
                    if last_recv is not None:
                        gap = now - last_recv
                        gaps.append(gap)
                        if gap > out["max_gap_s"]:
                            out["max_gap_s"] = round(gap, 3)
                    last_recv = now
                    out["frames"] += 1
                    try:
                        obj = json.loads(raw)
                    except json.JSONDecodeError:
                        out["errors"].append("non-json frame")
                        continue
                    if not isinstance(obj, dict):
                        continue
                    typ = str(obj.get("type") or "")
                    if typ and typ != expect_type:
                        out["adapter_notes"].append(f"unexpected type={typ}")

                    stations = obj.get("stations")
                    if isinstance(stations, list) and stations:
                        meta_n = len(stations)
                        out["meta_n"] = meta_n
                        # 适配器需要 code/lat/lon（或缺 lat 则跳过该点）
                        sample = stations[0] if stations else {}
                        if isinstance(sample, dict) and (
                            "code" in sample or "lat" in sample or "latitude" in sample
                        ):
                            out["meta_ok"] = True

                    data = obj.get(data_key)
                    if isinstance(data, list):
                        out["data_frames"] += 1
                        if meta_n > 0:
                            if len(data) == meta_n:
                                out["data_len_match"] += 1
                            else:
                                out["data_len_mismatch"] += 1
                        if need_time:
                            if obj.get("time") is not None:
                                out["time_field_ok"] += 1
                            ts = obj.get("timestamp")
                            if isinstance(ts, (int, float)):
                                out["timestamp_ok"] += 1
                                its = int(ts)
                                if last_ts is not None and its + 50 < last_ts:
                                    # 允许小幅乱序；大幅回退记异常
                                    out["timestamp_monotonic"] = False
                                last_ts = its
        except Exception as e:  # noqa: BLE001
            out["reconnects"] += 1
            out["errors"].append(str(e)[:200])
            await asyncio.sleep(backoff)
            backoff = min(backoff * 2, 15.0)

    if gaps:
        out["avg_gap_s"] = round(sum(gaps) / len(gaps), 3)
    out["max_gap_s"] = round(float(out["max_gap_s"]), 3)

    # 合格标准：有台账、有数据帧、长度对齐、平均间隔 < 3s、最大间隔 < 15s、重连少
    ok = (
        out["meta_ok"]
        and out["data_frames"] >= max(5, int(duration_s * 0.3))
        and out["data_len_mismatch"] == 0
        and out["avg_gap_s"] > 0
        and out["avg_gap_s"] < 3.0
        and out["max_gap_s"] < 15.0
        and out["reconnects"] <= 2
    )
    if need_time:
        ok = ok and out["time_field_ok"] >= max(3, out["data_frames"] // 2)
        ok = ok and out["timestamp_ok"] >= max(3, out["data_frames"] // 2)
        ok = ok and out["timestamp_monotonic"]
    out["ok"] = ok
    out["errors"] = out["errors"][-12:]
    out["adapter_notes"] = out["adapter_notes"][:20]
    return out


async def amain() -> int:
    duration = _secs()
    print(f"verify stations duration={duration}s (no auth)")
    results = []
    # 串行降低本机 FD/并发压力；三条彼此独立
    for name, spec in ENDPOINTS.items():
        print(f"  probing {name} ...")
        r = await verify_one(name, spec, duration)
        results.append(r)
        print(
            f"  {name}: ok={r['ok']} frames={r['frames']} meta={r['meta_n']} "
            f"data={r['data_frames']} match={r['data_len_match']} "
            f"gap_avg={r['avg_gap_s']}s gap_max={r['max_gap_s']}s "
            f"reconnects={r['reconnects']} "
            f"time_ok={r.get('time_field_ok')} ts_ok={r.get('timestamp_ok')}"
        )

    logs = ROOT / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    path = logs / f"station_verify_{stamp}.json"
    report = {
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "duration_s": duration,
        "results": results,
        "all_ok": all(r["ok"] for r in results),
    }
    path.write_bytes(json.dumps(report, ensure_ascii=False, indent=2).encode("utf-8"))
    print(f"report -> {path.relative_to(ROOT)} all_ok={report['all_ok']}")
    return 0 if report["all_ok"] else 1


def main() -> int:
    try:
        import websockets  # noqa: F401
    except ImportError:
        print("need: pip install websockets", file=sys.stderr)
        return 2
    return asyncio.run(amain())


if __name__ == "__main__":
    sys.exit(main())
