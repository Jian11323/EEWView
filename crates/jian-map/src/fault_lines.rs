//! 活动断层迹线：全球 GEM + 中国 CAFD（CN-faults），平面 / 球体同绘。

use crate::projection::{self, ViewCtx};
use egui::{Color32, Stroke, Ui};
use serde::Deserialize;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tracing::{info, warn};

#[derive(Clone)]
struct FaultSeg {
    /// 轴对齐包围盒：[min_lon, min_lat, max_lon, max_lat]
    bbox: [f32; 4],
    line: Vec<[f32; 2]>,
}

struct PaintCache {
    key: u64,
    shapes: Vec<egui::Shape>,
}

#[derive(Default)]
pub struct FaultLines {
    global: Vec<FaultSeg>,
    china: Vec<FaultSeg>,
    loaded: bool,
    paint_cache: Option<PaintCache>,
}

#[derive(Deserialize)]
struct Fc {
    features: Vec<Feat>,
}

#[derive(Deserialize)]
struct Feat {
    #[serde(default)]
    geometry: Option<Geom>,
}

#[derive(Deserialize)]
struct Geom {
    #[serde(rename = "type")]
    typ: String,
    coordinates: serde_json::Value,
}

impl FaultLines {
    pub fn load_dir(&mut self, dir: &Path) {
        if self.loaded {
            return;
        }
        let faults = dir.join("faults");
        self.global = load_file(&faults.join("global_active_faults.geojson"), "global faults");
        self.china = load_file(&faults.join("china_active_faults.geojson"), "china faults");
        self.loaded = true;
        self.paint_cache = None;
    }

    pub fn paint(&mut self, ui: &mut Ui, view: &ViewCtx, dark: bool) {
        if self.global.is_empty() && self.china.is_empty() {
            return;
        }

        let key = paint_key(view, dark);
        if let Some(c) = self.paint_cache.as_ref() {
            if c.key == key {
                ui.painter_at(view.rect).extend(c.shapes.clone());
                return;
            }
        }

        let (min_lon, min_lat, max_lon, max_lat) = view_lonlat_bounds(view);
        let step = lod_step(view.zoom);
        let (global_stroke, china_stroke) = strokes(dark, view.zoom);

        let mut shapes = Vec::with_capacity(2048);
        paint_layer(
            &mut shapes,
            &self.global,
            view,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            step,
            global_stroke,
        );
        // 近距叠全国 CAFD；远距仍画（GEM 华东偏稀）
        let china_step = if view.zoom < 4.0 { step.max(2) } else { step };
        paint_layer(
            &mut shapes,
            &self.china,
            view,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            china_step,
            china_stroke,
        );

        ui.painter_at(view.rect).extend(shapes.clone());
        self.paint_cache = Some(PaintCache { key, shapes });
    }
}

fn load_file(path: &Path, label: &str) -> Vec<FaultSeg> {
    if !path.is_file() {
        warn!(path = %path.display(), "缺少断层 GeoJSON（{label}）");
        return Vec::new();
    }
    match std::fs::read_to_string(path) {
        Ok(s) => match serde_json::from_str::<Fc>(&s) {
            Ok(fc) => {
                let segs: Vec<FaultSeg> = fc
                    .features
                    .into_iter()
                    .filter_map(|f| {
                        let geom = f.geometry.as_ref()?;
                        parse_lines(geom)
                    })
                    .flatten()
                    .filter_map(|line| {
                        if line.len() < 2 {
                            return None;
                        }
                        Some(FaultSeg {
                            bbox: line_bbox(&line),
                            line,
                        })
                    })
                    .collect();
                info!(n = segs.len(), path = %path.display(), %label, "loaded faults");
                segs
            }
            Err(e) => {
                warn!(error = %e, path = %path.display(), "断层 GeoJSON 解析失败");
                Vec::new()
            }
        },
        Err(e) => {
            warn!(error = %e, path = %path.display(), "读取断层 GeoJSON 失败");
            Vec::new()
        }
    }
}

fn line_bbox(line: &[[f32; 2]]) -> [f32; 4] {
    let mut min_lon = f32::MAX;
    let mut min_lat = f32::MAX;
    let mut max_lon = f32::MIN;
    let mut max_lat = f32::MIN;
    for p in line {
        min_lon = min_lon.min(p[0]);
        max_lon = max_lon.max(p[0]);
        min_lat = min_lat.min(p[1]);
        max_lat = max_lat.max(p[1]);
    }
    [min_lon, min_lat, max_lon, max_lat]
}

fn bbox_overlap(b: &[f32; 4], min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> bool {
    // 平面图跨日界线时视口 lon 可能 wrap；简单两段检测
    let overlaps = |a0: f64, a1: f64, b0: f64, b1: f64| a0 <= b1 && b0 <= a1;
    let lat_ok = overlaps(b[1] as f64, b[3] as f64, min_lat, max_lat);
    if !lat_ok {
        return false;
    }
    if min_lon <= max_lon {
        overlaps(b[0] as f64, b[2] as f64, min_lon, max_lon)
    } else {
        // 视口跨 ±180：[min_lon,180] U [-180,max_lon]
        overlaps(b[0] as f64, b[2] as f64, min_lon, 180.0)
            || overlaps(b[0] as f64, b[2] as f64, -180.0, max_lon)
    }
}

fn view_lonlat_bounds(view: &ViewCtx) -> (f64, f64, f64, f64) {
    if view.mode.is_globe() {
        // 半球外接粗估：按 zoom 放大可视角
        let half = (90.0 / view.zoom.max(2.0)).clamp(25.0, 95.0);
        let min_lat = (view.center_lat - half).max(-90.0);
        let max_lat = (view.center_lat + half).min(90.0);
        let mut min_lon = view.center_lon - half / view.center_lat.to_radians().cos().abs().max(0.2);
        let mut max_lon = view.center_lon + half / view.center_lat.to_radians().cos().abs().max(0.2);
        min_lon = projection::normalize_lon(min_lon);
        max_lon = projection::normalize_lon(max_lon);
        (min_lon, min_lat, max_lon, max_lat)
    } else {
        let corners = [
            view.screen_to_lonlat(view.rect.left_top()),
            view.screen_to_lonlat(view.rect.right_top()),
            view.screen_to_lonlat(view.rect.left_bottom()),
            view.screen_to_lonlat(view.rect.right_bottom()),
        ];
        let mut min_lon = f64::INFINITY;
        let mut max_lon = f64::NEG_INFINITY;
        let mut min_lat = f64::INFINITY;
        let mut max_lat = f64::NEG_INFINITY;
        for (lon, lat) in corners {
            min_lon = min_lon.min(lon);
            max_lon = max_lon.max(lon);
            min_lat = min_lat.min(lat);
            max_lat = max_lat.max(lat);
        }
        // 跨日界：若角点 lon 跨度很大，用 center±半宽
        if max_lon - min_lon > 300.0 {
            let half_w = (view.rect.width() as f64)
                / projection::world_size(view.zoom)
                * 180.0
                + 8.0;
            (
                projection::normalize_lon(view.center_lon - half_w),
                min_lat - 2.0,
                projection::normalize_lon(view.center_lon + half_w),
                max_lat + 2.0,
            )
        } else {
            (min_lon - 2.0, min_lat - 2.0, max_lon + 2.0, max_lat + 2.0)
        }
    }
}

fn lod_step(zoom: f64) -> usize {
    if zoom < 3.0 {
        4
    } else if zoom < 4.5 {
        2
    } else {
        1
    }
}

fn strokes(dark: bool, zoom: f64) -> (Stroke, Stroke) {
    let w = if zoom < 4.0 {
        0.7_f32
    } else if zoom < 6.5 {
        0.95_f32
    } else {
        1.15_f32
    };
    if dark {
        (
            Stroke::new(w, Color32::from_rgba_unmultiplied(0xC4, 0x6A, 0x4A, 160)),
            Stroke::new(
                w + 0.15_f32,
                Color32::from_rgba_unmultiplied(0xE0, 0x7A, 0x52, 200),
            ),
        )
    } else {
        (
            Stroke::new(w, Color32::from_rgba_unmultiplied(0x9A, 0x3E, 0x2A, 170)),
            Stroke::new(
                w + 0.15_f32,
                Color32::from_rgba_unmultiplied(0xB5, 0x45, 0x2E, 210),
            ),
        )
    }
}

fn paint_key(view: &ViewCtx, dark: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.mode.hash(&mut h);
    ((view.center_lon * 200.0).round() as i64).hash(&mut h);
    ((view.center_lat * 200.0).round() as i64).hash(&mut h);
    ((view.zoom * 40.0).round() as i64).hash(&mut h);
    ((view.rect.width() * 0.5).round() as i32).hash(&mut h);
    ((view.rect.height() * 0.5).round() as i32).hash(&mut h);
    dark.hash(&mut h);
    h.finish()
}

fn paint_layer(
    shapes: &mut Vec<egui::Shape>,
    segs: &[FaultSeg],
    view: &ViewCtx,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    step: usize,
    stroke: Stroke,
) {
    let step = step.max(1);
    for seg in segs {
        if !bbox_overlap(&seg.bbox, min_lon, min_lat, max_lon, max_lat) {
            continue;
        }
        if step == 1 {
            paint_line(shapes, &seg.line, view, stroke);
        } else {
            let mut thin = Vec::with_capacity(seg.line.len() / step + 2);
            for (i, p) in seg.line.iter().enumerate() {
                if i % step == 0 || i + 1 == seg.line.len() {
                    thin.push(*p);
                }
            }
            if thin.len() >= 2 {
                paint_line(shapes, &thin, view, stroke);
            }
        }
    }
}

fn parse_lines(g: &Geom) -> Option<Vec<Vec<[f32; 2]>>> {
    match g.typ.as_str() {
        "LineString" => {
            let pts = coords_line(&g.coordinates)?;
            Some(vec![pts])
        }
        "MultiLineString" => {
            let arr = g.coordinates.as_array()?;
            let mut out = Vec::new();
            for line in arr {
                if let Some(pts) = coords_line(line) {
                    out.push(pts);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

fn coords_line(v: &serde_json::Value) -> Option<Vec<[f32; 2]>> {
    let arr = v.as_array()?;
    let mut pts = Vec::with_capacity(arr.len());
    for p in arr {
        let xy = p.as_array()?;
        if xy.len() < 2 {
            continue;
        }
        let lon = xy[0].as_f64()? as f32;
        let lat = xy[1].as_f64()? as f32;
        pts.push([lon, lat]);
    }
    if pts.len() < 2 {
        None
    } else {
        Some(pts)
    }
}

fn paint_line(
    shapes: &mut Vec<egui::Shape>,
    line: &[[f32; 2]],
    view: &ViewCtx,
    stroke: Stroke,
) {
    let mut pts = Vec::with_capacity(line.len());
    let mut last: Option<egui::Pos2> = None;
    let max_jump = if view.mode.is_globe() {
        view.horizon_radius().unwrap_or(80.0) * 0.35
    } else {
        projection::world_size(view.zoom) as f32 * (80.0 / 360.0)
    };
    for p in line {
        let Some(sp) = view.project(p[0] as f64, p[1] as f64) else {
            flush_path(shapes, &mut pts, stroke);
            last = None;
            continue;
        };
        if let Some(prev) = last {
            let dx = sp.x - prev.x;
            let dy = sp.y - prev.y;
            if dx * dx + dy * dy > max_jump * max_jump {
                flush_path(shapes, &mut pts, stroke);
            }
        }
        pts.push(sp);
        last = Some(sp);
    }
    flush_path(shapes, &mut pts, stroke);
}

fn flush_path(shapes: &mut Vec<egui::Shape>, pts: &mut Vec<egui::Pos2>, stroke: Stroke) {
    if pts.len() >= 2 {
        shapes.push(
            egui::epaint::PathShape {
                points: std::mem::take(pts),
                closed: false,
                fill: Color32::TRANSPARENT,
                stroke: egui::epaint::PathStroke::new(stroke.width, stroke.color),
            }
            .into(),
        );
    } else {
        pts.clear();
    }
}
