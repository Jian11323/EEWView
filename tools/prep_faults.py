# -*- coding: utf-8 -*-
"""下载并简化活动断层 GeoJSON 到 geodata/faults/。

产出：
- `*_active_faults.geojson`：全部分段（近距 fine）
- `*_active_faults_lod0.geojson`：长断层（远距）
- `*_active_faults_lod1.geojson`：中等断层（中距）

也可对已有 fine 只生成 LOD：`python tools/prep_faults.py --lod-only`
"""
from __future__ import annotations

import argparse
import json
import math
import re
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FAULTS = ROOT / "geodata" / "faults"

GEM_URL = (
    "https://raw.githubusercontent.com/GEMScienceTools/gem-global-active-faults/"
    "master/geojson/gem_active_faults.geojson"
)
CN_URL = (
    "https://raw.githubusercontent.com/gmt-china/china-geospatial-data/main/CN-faults.gmt"
)

# 与 jian-map fault_lines 阈值对齐
LOD0_MIN_DIAG = 1.0
LOD1_MIN_DIAG = 0.28


def download(url: str, dest: Path) -> None:
    print(f"download {url}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    with urllib.request.urlopen(url, timeout=120) as r:
        dest.write_bytes(r.read())
    print(f"  -> {dest} ({dest.stat().st_size} bytes)")


def parse_ogr_gmt(path: Path) -> list[dict]:
    text = path.read_text(encoding="utf-8", errors="replace")
    features: list[dict] = []
    chunks = re.split(r"(?m)^>\s*$", text)
    for chunk in chunks:
        lines = [ln.strip() for ln in chunk.splitlines() if ln.strip()]
        if not lines:
            continue
        props: dict = {}
        coords: list[list[float]] = []
        for ln in lines:
            if ln.startswith("#"):
                if ln.startswith("# @N") or ln.startswith("#@N"):
                    props["_names"] = ln.split("N", 1)[1].strip().split("|")
                elif ln.startswith("# @D") or ln.startswith("#@D"):
                    vals = ln.split("D", 1)[1].strip().split("|")
                    names = props.pop("_names", None)
                    if isinstance(names, list):
                        for i, name in enumerate(names):
                            if i < len(vals) and name:
                                props[name] = vals[i]
                continue
            parts = ln.split()
            if len(parts) >= 2:
                try:
                    lon = float(parts[0])
                    lat = float(parts[1])
                except ValueError:
                    continue
                if -180.0 <= lon <= 180.0 and -90.0 <= lat <= 90.0:
                    coords.append([lon, lat])
        if len(coords) >= 2:
            features.append(
                {
                    "type": "Feature",
                    "properties": {k: v for k, v in props.items() if not str(k).startswith("_")},
                    "geometry": {"type": "LineString", "coordinates": coords},
                }
            )
    return features


def simplify_line(coords: list[list[float]], tol_deg: float) -> list[list[float]]:
    if len(coords) <= 2 or tol_deg <= 0:
        return coords

    def perp_dist(p, a, b):
        ax, ay = a
        bx, by = b
        px, py = p
        dx, dy = bx - ax, by - ay
        if dx == 0 and dy == 0:
            return math.hypot(px - ax, py - ay)
        t = ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)
        t = max(0.0, min(1.0, t))
        return math.hypot(px - (ax + t * dx), py - (ay + t * dy))

    def dp(pts):
        if len(pts) <= 2:
            return pts
        a, b = pts[0], pts[-1]
        idx, mx = 0, 0.0
        for i in range(1, len(pts) - 1):
            d = perp_dist(pts[i], a, b)
            if d > mx:
                idx, mx = i, d
        if mx > tol_deg:
            left = dp(pts[: idx + 1])
            right = dp(pts[idx:])
            return left[:-1] + right
        return [a, b]

    return dp(coords)


def extract_lines(geom: dict) -> list[list[list[float]]]:
    t = geom.get("type")
    c = geom.get("coordinates")
    if not t or c is None:
        return []
    if t == "LineString":
        return [c] if len(c) >= 2 else []
    if t == "MultiLineString":
        return [ln for ln in c if len(ln) >= 2]
    return []


def line_diag(coords: list[list[float]]) -> float:
    if len(coords) < 2:
        return 0.0
    xs = [p[0] for p in coords]
    ys = [p[1] for p in coords]
    return math.hypot(max(xs) - min(xs), max(ys) - min(ys))


def slim_collection(features: list[dict], tol: float, source: str) -> dict:
    out = []
    n_in = n_pts_in = n_pts_out = 0
    for f in features:
        geom = f.get("geometry") or {}
        for line in extract_lines(geom):
            n_in += 1
            n_pts_in += len(line)
            slim = simplify_line(
                [[float(p[0]), float(p[1])] for p in line if len(p) >= 2], tol
            )
            if len(slim) < 2:
                continue
            n_pts_out += len(slim)
            out.append(
                {
                    "type": "Feature",
                    "properties": {"src": source},
                    "geometry": {"type": "LineString", "coordinates": slim},
                }
            )
    print(f"{source}: lines={n_in} pts {n_pts_in}->{n_pts_out} out={len(out)}")
    return {"type": "FeatureCollection", "features": out}


def write_fc(path: Path, fc: dict) -> None:
    path.write_text(
        json.dumps(fc, ensure_ascii=False, separators=(",", ":")), encoding="utf-8"
    )
    print(f"wrote {path} ({path.stat().st_size} bytes)")


def emit_lod_from_fine(fine_path: Path, stem: str, source: str) -> None:
    """从 fine FeatureCollection 写出互斥 lod0/lod1（lod2 即 fine 全量）。"""
    fc = json.loads(fine_path.read_text(encoding="utf-8"))
    feats = fc.get("features") or []
    lod0, lod1 = [], []
    for f in feats:
        geom = f.get("geometry") or {}
        lines = extract_lines(geom)
        if not lines:
            continue
        # 取最长一段代表 diag
        best = max(lines, key=lambda ln: line_diag(ln))
        diag = line_diag(best)
        props = dict(f.get("properties") or {})
        props["src"] = source
        if diag >= LOD0_MIN_DIAG:
            slim = simplify_line(
                [[float(p[0]), float(p[1])] for p in best if len(p) >= 2], 0.05
            )
            if len(slim) >= 2:
                lod0.append(
                    {
                        "type": "Feature",
                        "properties": props,
                        "geometry": {"type": "LineString", "coordinates": slim},
                    }
                )
        elif diag >= LOD1_MIN_DIAG:
            slim = simplify_line(
                [[float(p[0]), float(p[1])] for p in best if len(p) >= 2], 0.02
            )
            if len(slim) >= 2:
                lod1.append(
                    {
                        "type": "Feature",
                        "properties": props,
                        "geometry": {"type": "LineString", "coordinates": slim},
                    }
                )
    write_fc(
        FAULTS / f"{stem}_lod0.geojson",
        {"type": "FeatureCollection", "features": lod0},
    )
    write_fc(
        FAULTS / f"{stem}_lod1.geojson",
        {"type": "FeatureCollection", "features": lod1},
    )
    print(f"{stem} lod: lod0={len(lod0)} lod1={len(lod1)} fine={len(feats)}")


def emit_all_lod() -> None:
    emit_lod_from_fine(
        FAULTS / "global_active_faults.geojson", "global_active_faults", "gem"
    )
    emit_lod_from_fine(
        FAULTS / "china_active_faults.geojson", "china_active_faults", "cn"
    )


def main() -> None:
    ap = argparse.ArgumentParser(description="Prepare active fault GeoJSON + LOD")
    ap.add_argument(
        "--lod-only",
        action="store_true",
        help="只从已有 fine GeoJSON 生成 lod0/lod1，不下载上游",
    )
    args = ap.parse_args()
    FAULTS.mkdir(parents=True, exist_ok=True)

    if args.lod_only:
        emit_all_lod()
        print("done (lod-only)")
        return

    raw_gem = FAULTS / "_raw_gem_active_faults.geojson"
    raw_cn = FAULTS / "_raw_CN-faults.gmt"

    download(GEM_URL, raw_gem)
    download(CN_URL, raw_cn)

    cn_feats = parse_ogr_gmt(raw_cn)
    if not cn_feats:
        raise SystemExit("CN-faults parse produced 0 features")
    write_fc(FAULTS / "china_active_faults.geojson", slim_collection(cn_feats, 0.008, "cn"))

    gem = json.loads(raw_gem.read_text(encoding="utf-8"))
    write_fc(
        FAULTS / "global_active_faults.geojson",
        slim_collection(gem.get("features") or [], 0.02, "gem"),
    )

    raw_gem.unlink(missing_ok=True)
    raw_cn.unlink(missing_ok=True)
    emit_all_lod()
    print("done (raw intermediates removed)")


if __name__ == "__main__":
    main()
