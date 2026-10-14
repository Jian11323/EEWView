# -*- coding: utf-8 -*-
"""真实上游轻量压测（可选）。

默认跳过。显式开启：
  set EEWVIEW_STRESS=1
  python tools/stress_live_sources.py

可选环境变量：
  EEWVIEW_STRESS_SECS   运行秒数，默认 120
  EEWVIEW_STRESS_WS     1=尝试 WebSocket（需 pip install websockets），默认 1

读取 config/default.toml（及同目录 local.toml 若存在，不入库密钥）。
报告写入 logs/stress_live_YYYYMMDD_HHMMSS.json（勿提交）。
"""
from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_EQLIST = "https://api.wolfx.jp/cenc_eqlist.json"
DEFAULT_WS = "wss://ws-api.wolfx.jp/all_eew"
DEFAULT_P2P = "https://api.p2pquake.net/v2/history?codes=551&limit=10"


def load_toml_urls() -> tuple[str, str, bool, bool]:
    """返回 (eqlist_url, ws_url, wolfx_on, p2p_on)。"""
    eqlist = DEFAULT_EQLIST
    ws = DEFAULT_WS
    wolfx_on = True
    p2p_on = True
    cfg_path = ROOT / "config" / "default.toml"
    text = ""
    if cfg_path.is_file():
        text = cfg_path.read_text(encoding="utf-8")
    local = ROOT / "config" / "local.toml"
    if local.is_file():
        text += "\n" + local.read_text(encoding="utf-8")
    # 粗解析，避免强依赖 tomllib 版本差异外的完整合并
    if "cenc_eqlist.json" in text or "http_eqlist" in text:
        for line in text.splitlines():
            s = line.strip()
            if s.startswith("http_eqlist"):
                # http_eqlist = "..."
                if '"' in s:
                    eqlist = s.split('"', 2)[1]
            if s.startswith("enabled") and "wolfx" in text:
                pass
    # ws_urls 默认即可；enabled 粗检
    in_wolfx = False
    in_p2p = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[sources.wolfx]"):
            in_wolfx, in_p2p = True, False
            continue
        if s.startswith("[sources.p2pquake]"):
            in_p2p, in_wolfx = True, False
            continue
        if s.startswith("["):
            in_wolfx = in_p2p = False
            continue
        if s.startswith("enabled"):
            val = s.split("=", 1)[-1].strip().lower()
            on = val in ("true", "1", "yes")
            if in_wolfx:
                wolfx_on = on
            if in_p2p:
                p2p_on = on
        if s.startswith("ws_urls") and '"' in s:
            # ws_urls = ["wss://..."] 单元素常见
            parts = s.split('"')
            if len(parts) >= 2:
                ws = parts[1]
    return eqlist, ws, wolfx_on, p2p_on


def http_get(url: str, timeout: float = 20.0) -> tuple[bool, int, str]:
    req = urllib.request.Request(
        url,
        headers={"User-Agent": "EEWView-stress/1.3 (+https://github.com/Jian11323/EEWView)"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            body = resp.read()
            return True, resp.status, body[:200].decode("utf-8", errors="replace")
    except urllib.error.HTTPError as e:
        return False, int(e.code), str(e.reason)
    except Exception as e:  # noqa: BLE001 — 压测汇总错误
        return False, 0, str(e)


def stress_http(url: str, label: str, duration_s: float, interval_s: float) -> dict:
    ok = fail = 0
    parse_ok = parse_fail = 0
    t0 = time.monotonic()
    samples: list[dict] = []
    while time.monotonic() - t0 < duration_s:
        req = urllib.request.Request(
            url,
            headers={"User-Agent": "EEWView-stress/1.3 (+https://github.com/Jian11323/EEWView)"},
        )
        good = False
        status = 0
        snippet = ""
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                status = resp.status
                raw = resp.read()
                snippet = raw[:80].decode("utf-8", errors="replace")
                good = True
                ok += 1
                try:
                    data = json.loads(raw.decode("utf-8", errors="replace"))
                    if isinstance(data, (dict, list)):
                        parse_ok += 1
                    else:
                        parse_fail += 1
                except json.JSONDecodeError:
                    parse_fail += 1
        except urllib.error.HTTPError as e:
            fail += 1
            status = int(e.code)
            snippet = str(e.reason)
        except Exception as e:  # noqa: BLE001
            fail += 1
            snippet = str(e)
        samples.append(
            {
                "t": round(time.monotonic() - t0, 2),
                "ok": good,
                "status": status,
                "snippet": snippet[:80],
            }
        )
        time.sleep(interval_s)
    return {
        "label": label,
        "url": url,
        "http_ok": ok,
        "http_fail": fail,
        "parse_ok": parse_ok,
        "parse_fail": parse_fail,
        "samples": samples[-20:],
    }


def stress_ws(url: str, duration_s: float) -> dict:
    try:
        import asyncio

        import websockets  # type: ignore
    except ImportError:
        return {
            "label": "wolfx_ws",
            "url": url,
            "skipped": True,
            "reason": "websockets not installed (pip install websockets)",
        }

    stats = {
        "label": "wolfx_ws",
        "url": url,
        "messages": 0,
        "text_frames": 0,
        "json_ok": 0,
        "json_fail": 0,
        "reconnects": 0,
        "errors": [],
    }

    async def run() -> None:
        deadline = time.monotonic() + duration_s
        backoff = 1.0
        while time.monotonic() < deadline:
            try:
                async with websockets.connect(
                    url,
                    open_timeout=15,
                    ping_interval=20,
                    ping_timeout=20,
                    max_size=4 * 1024 * 1024,
                ) as ws:
                    backoff = 1.0
                    await ws.send("ping")
                    await ws.send("query_cenceqlist")
                    while time.monotonic() < deadline:
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            break
                        try:
                            msg = await asyncio.wait_for(ws.recv(), timeout=min(30.0, remaining))
                        except asyncio.TimeoutError:
                            await ws.send("ping")
                            continue
                        stats["messages"] += 1
                        if isinstance(msg, str):
                            stats["text_frames"] += 1
                            try:
                                json.loads(msg)
                                stats["json_ok"] += 1
                            except json.JSONDecodeError:
                                # ping/pong 文本不算失败
                                if msg.strip().lower() not in ("pong", "ping", "ok"):
                                    stats["json_fail"] += 1
            except Exception as e:  # noqa: BLE001
                stats["reconnects"] += 1
                stats["errors"].append(str(e)[:200])
                await asyncio.sleep(backoff)
                backoff = min(backoff * 2, 30.0)

    asyncio.run(run())
    stats["errors"] = stats["errors"][-10:]
    return stats


def main() -> int:
    if os.environ.get("EEWVIEW_STRESS", "").strip() not in ("1", "true", "TRUE", "yes"):
        print("stress_live_sources: skip (set EEWVIEW_STRESS=1 to run)")
        return 0

    duration = float(os.environ.get("EEWVIEW_STRESS_SECS", "120"))
    use_ws = os.environ.get("EEWVIEW_STRESS_WS", "1").strip() not in ("0", "false", "FALSE")
    eqlist, ws_url, wolfx_on, p2p_on = load_toml_urls()

    logs = ROOT / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d_%H%M%S")
    report_path = logs / f"stress_live_{stamp}.json"

    report: dict = {
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "duration_s": duration,
        "wolfx_enabled_cfg": wolfx_on,
        "p2p_enabled_cfg": p2p_on,
        "results": [],
    }

    print(f"stress: duration={duration}s wolfx={wolfx_on} p2p={p2p_on}")

    # HTTP 轮询压测：各源并行时间片（顺序跑，总时长约 duration）
    slice_s = max(15.0, duration / (2 + (1 if use_ws and wolfx_on else 0)))

    if wolfx_on:
        print(f"  HTTP wolfx eqlist {eqlist}")
        report["results"].append(stress_http(eqlist, "wolfx_eqlist", slice_s, 8.0))

    if p2p_on:
        print(f"  HTTP p2pquake {DEFAULT_P2P}")
        report["results"].append(stress_http(DEFAULT_P2P, "p2pquake", slice_s, 10.0))

    if use_ws and wolfx_on:
        print(f"  WS wolfx {ws_url}")
        report["results"].append(stress_ws(ws_url, slice_s))

    report["finished_utc"] = datetime.now(timezone.utc).isoformat()
    report_path.write_bytes(json.dumps(report, ensure_ascii=False, indent=2).encode("utf-8"))
    print(f"stress: report -> {report_path.relative_to(ROOT)}")

    # 失败判定：HTTP 全失败则 exit 1
    hard_fail = False
    for r in report["results"]:
        if r.get("skipped"):
            continue
        if "http_ok" in r and r["http_ok"] == 0 and r.get("http_fail", 0) > 0:
            hard_fail = True
            print(f"  FAIL {r.get('label')}: all HTTP attempts failed")
    return 1 if hard_fail else 0


if __name__ == "__main__":
    sys.exit(main())
