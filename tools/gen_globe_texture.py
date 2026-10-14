# -*- coding: utf-8 -*-
"""Generate equirectangular filled land texture for globe basemap."""
from __future__ import annotations

import json
import math
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
GEODATA = ROOT / "geodata"
OUT = GEODATA / "globe_dark.png"

# 2x 绘制再缩到输出尺寸，减轻锯齿
DRAW_W, DRAW_H = 8192, 4096
OUT_W, OUT_H = 4096, 2048
OCEAN = (5, 20, 30, 255)
LAND = (28, 72, 98, 255)


def lonlat_to_xy(lon: float, lat: float) -> tuple[float, float]:
    x = (lon + 180.0) / 360.0 * DRAW_W
    y = (90.0 - lat) / 180.0 * DRAW_H
    return x, y


def draw_ring(draw: ImageDraw.ImageDraw, ring: list, fill) -> None:
    if len(ring) < 3:
        return
    raw = []
    for p in ring:
        if not isinstance(p, (list, tuple)) or len(p) < 2:
            continue
        lon, lat = float(p[0]), float(p[1])
        if not math.isfinite(lon) or not math.isfinite(lat):
            continue
        raw.append((lon, lat))
    if len(raw) < 3:
        return
    unwrapped = [raw[0]]
    for lon, lat in raw[1:]:
        prev = unwrapped[-1][0]
        while lon - prev > 180.0:
            lon -= 360.0
        while prev - lon > 180.0:
            lon += 360.0
        unwrapped.append((lon, lat))
    # ±360° 副本，避免日界线竖缝
    for shift in (-360.0, 0.0, 360.0):
        pts = [lonlat_to_xy(lon + shift, lat) for lon, lat in unwrapped]
        if all(x < -16 or x > DRAW_W + 16 for x, _ in pts):
            continue
        if len(pts) >= 3:
            draw.polygon(pts, fill=fill)


def iter_geojson_polygons(path: Path):
    data = json.loads(path.read_text(encoding="utf-8"))
    feats = data.get("features") or []
    if data.get("type") == "Feature":
        feats = [data]
    if data.get("type") == "GeometryCollection":
        for g in data.get("geometries") or []:
            yield from geom_polys(g)
        return
    for f in feats:
        g = f.get("geometry") if isinstance(f, dict) else None
        if g:
            yield from geom_polys(g)


def geom_polys(g: dict):
    t = g.get("type")
    c = g.get("coordinates")
    if not c:
        return
    if t == "Polygon":
        yield c
    elif t == "MultiPolygon":
        for poly in c:
            yield poly


def decode_topo_region(path: Path):
    topo = json.loads(path.read_text(encoding="utf-8"))
    arcs = topo["arcs"]
    scale = topo["transform"]["scale"]
    translate = topo["transform"]["translate"]

    def decode_arc(idx: int):
        sign = -1 if idx < 0 else 1
        i = ~idx if idx < 0 else idx
        arc = arcs[i]
        x = y = 0
        out = []
        for dx, dy in arc:
            x += dx
            y += dy
            lon = x * scale[0] + translate[0]
            lat = y * scale[1] + translate[1]
            out.append([lon, lat])
        if sign < 0:
            out.reverse()
        return out

    def ring_from_indices(indices):
        pts = []
        for a in indices:
            seg = decode_arc(int(a))
            if pts and seg:
                seg = seg[1:]
            pts.extend(seg)
        return pts

    obj = topo["objects"].get("region") or next(iter(topo["objects"].values()))
    for geom in obj.get("geometries") or []:
        arcs_v = geom.get("arcs")
        if not arcs_v:
            continue
        gt = geom.get("type")
        if gt == "Polygon":
            yield [ring_from_indices(r) for r in arcs_v]
        elif gt == "MultiPolygon":
            for poly in arcs_v:
                yield [ring_from_indices(r) for r in poly]


def paint_polys(draw, polys_iter, fill):
    n = 0
    for poly in polys_iter:
        if not poly:
            continue
        draw_ring(draw, poly[0], fill)
        n += 1
    return n


def main():
    img = Image.new("RGBA", (DRAW_W, DRAW_H), OCEAN)
    draw = ImageDraw.Draw(img)

    gq = GEODATA / "globalquake"
    world = None
    for name in (
        "countriesHD.json",
        "countriesMD.json",
        "countriesUHD.json",
    ):
        p = gq / name
        if p.is_file():
            world = p
            break
    if world is None:
        for name in (
            "medium.global.modified.geo.json",
            "Globalmap.geo.json",
        ):
            p = GEODATA / name
            if p.is_file():
                world = p
                break
    if world is None:
        raise SystemExit("no world geojson")

    print("world", world)
    print("world polys", paint_polys(draw, iter_geojson_polygons(world), LAND))

    cn_gq = gq / "cn-prefectures.geojson"
    jp_gq = gq / "jp-prefectures.geojson"
    if cn_gq.is_file():
        print("cn polys", paint_polys(draw, iter_geojson_polygons(cn_gq), LAND))
    else:
        cn = GEODATA / "kanameishi" / "cn.province.topo.json"
        if cn.is_file():
            print("cn polys", paint_polys(draw, decode_topo_region(cn), LAND))

    if jp_gq.is_file():
        print("jp polys", paint_polys(draw, iter_geojson_polygons(jp_gq), LAND))
    else:
        jp = GEODATA / "kanameishi" / "jp.pref.topo.json"
        if jp.is_file():
            print("jp polys", paint_polys(draw, decode_topo_region(jp), LAND))

    OUT.parent.mkdir(parents=True, exist_ok=True)
    img = img.resize((OUT_W, OUT_H), Image.Resampling.LANCZOS)
    img.save(OUT, optimize=True)
    print("wrote", OUT, OUT.stat().st_size // 1024, "KB")


if __name__ == "__main__":
    main()
