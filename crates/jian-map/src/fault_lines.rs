//! 活动断层迹线：全球 GEM + 中国 CAFD（CN-faults），平面 / 球体同绘。
//!
//! 性能：空间网格索引 + 按缩放抽稀/短线剔除 + 绘制预算；缓存为单份 Mesh，
//! 避免每帧对上万条 PathShape 做描边镶嵌与 clone。

use crate::projection::{self, ViewCtx};
use egui::{Color32, Mesh, Pos2, Ui};
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tracing::{info, warn};

/// 网格单元边长（度）。约 5° 一格，全球约 36×36 格。
const GRID_CELL_DEG: f32 = 5.0;

/// 单帧最多绘制的线段条数（全球+中国合计）。
const MAX_PAINT_SEGS: usize = 5000;
/// 为中国 CAFD 预留条数，避免全球层先占满导致华东断层消失。
const CHINA_RESERVE: usize = 1500;

#[derive(Clone)]
struct FaultSeg {
    /// 轴对齐包围盒：[min_lon, min_lat, max_lon, max_lat]
    bbox: [f32; 4],
    /// bbox 对角线（度），供远距剔除短线
    diag: f32,
    line: Vec<[f32; 2]>,
}

#[derive(Default)]
struct SegLayer {
    segs: Vec<FaultSeg>,
    /// (ix, iy) → 落在该格的线段下标
    grid: HashMap<(i16, i16), Vec<u32>>,
}

struct PaintCache {
    key: u64,
    mesh: Mesh,
}

#[derive(Default)]
pub struct FaultLines {
    global: SegLayer,
    china: SegLayer,
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
        self.global = SegLayer::from_segs(load_file(
            &faults.join("global_active_faults.geojson"),
            "global faults",
        ));
        self.china = SegLayer::from_segs(load_file(
            &faults.join("china_active_faults.geojson"),
            "china faults",
        ));
        self.loaded = true;
        self.paint_cache = None;
    }

    pub fn paint(&mut self, ui: &mut Ui, view: &ViewCtx, dark: bool) {
        if self.global.segs.is_empty() && self.china.segs.is_empty() {
            return;
        }

        let ppp = ui.ctx().pixels_per_point();
        let key = paint_key(view, dark, ppp);
        if let Some(c) = self.paint_cache.as_ref() {
            if c.key == key {
                ui.painter_at(view.rect)
                    .add(egui::Shape::mesh(c.mesh.clone()));
                return;
            }
        }

        let (min_lon, min_lat, max_lon, max_lat) = view_lonlat_bounds(view);
        let step = lod_step(view.zoom);
        let stride_g = seg_stride(view.zoom, false);
        let stride_c = seg_stride(view.zoom, true);
        let min_diag = min_diag_deg(view.zoom);
        let (global_rgba, china_rgba, width) = stroke_style(dark, view.zoom);

        let mut mesh = Mesh::default();
        let global_budget = MAX_PAINT_SEGS.saturating_sub(CHINA_RESERVE);
        let painted_g = paint_layer(
            &mut mesh,
            &self.global,
            view,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            step,
            stride_g,
            min_diag,
            width,
            global_rgba,
            global_budget,
        );
        // 近距叠全国 CAFD；远距仍画但更狠抽稀（GEM 华东偏稀）
        // 中国层至少 CHINA_RESERVE，全球未用完的额度一并给中国
        let china_budget = CHINA_RESERVE + global_budget.saturating_sub(painted_g);
        let _ = paint_layer(
            &mut mesh,
            &self.china,
            view,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            if view.zoom < 4.0 { step.max(2) } else { step },
            stride_c,
            min_diag,
            width + 0.15,
            china_rgba,
            china_budget,
        );

        ui.painter_at(view.rect)
            .add(egui::Shape::mesh(mesh.clone()));
        self.paint_cache = Some(PaintCache { key, mesh });
    }
}

impl SegLayer {
    fn from_segs(segs: Vec<FaultSeg>) -> Self {
        let mut grid: HashMap<(i16, i16), Vec<u32>> = HashMap::new();
        for (i, seg) in segs.iter().enumerate() {
            let idx = i as u32;
            for cell in bbox_cells(&seg.bbox) {
                grid.entry(cell).or_default().push(idx);
            }
        }
        Self { segs, grid }
    }

    fn query_candidates(&self, min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Vec<u32> {
        let mut out = Vec::with_capacity(512);
        let mut seen = vec![false; self.segs.len()];
        // 格顺序固定，避免 HashMap 迭代导致预算内选段抖动
        let mut cells = view_cells(min_lon, min_lat, max_lon, max_lat);
        cells.sort_unstable();
        for cell in cells {
            let Some(list) = self.grid.get(&cell) else {
                continue;
            };
            for &idx in list {
                let i = idx as usize;
                if i < seen.len() && !seen[i] {
                    seen[i] = true;
                    out.push(idx);
                }
            }
        }
        // 长断层优先占用绘制预算，同长按索引稳定
        out.sort_by(|&a, &b| {
            let da = self.segs[a as usize].diag;
            let db = self.segs[b as usize].diag;
            db.total_cmp(&da).then_with(|| a.cmp(&b))
        });
        out
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
                        let bbox = line_bbox(&line);
                        let diag = ((bbox[2] - bbox[0]).hypot(bbox[3] - bbox[1])).max(0.0);
                        Some(FaultSeg { bbox, diag, line })
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

fn lon_cell(lon: f32) -> i16 {
    // [-180,180] → 格索引
    let x = ((lon + 180.0) / GRID_CELL_DEG).floor() as i32;
    x.clamp(0, (360.0 / GRID_CELL_DEG) as i32 - 1) as i16
}

fn lat_cell(lat: f32) -> i16 {
    let y = ((lat + 90.0) / GRID_CELL_DEG).floor() as i32;
    y.clamp(0, (180.0 / GRID_CELL_DEG) as i32 - 1) as i16
}

fn bbox_cells(b: &[f32; 4]) -> Vec<(i16, i16)> {
    let ix0 = lon_cell(b[0]);
    let ix1 = lon_cell(b[2]);
    let iy0 = lat_cell(b[1]);
    let iy1 = lat_cell(b[3]);
    let mut out = Vec::new();
    if ix0 <= ix1 {
        for ix in ix0..=ix1 {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
    } else {
        // 跨日界线
        let max_ix = lon_cell(180.0);
        for ix in ix0..=max_ix {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
        for ix in 0..=ix1 {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
    }
    out
}

fn view_cells(min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> Vec<(i16, i16)> {
    let iy0 = lat_cell(min_lat as f32);
    let iy1 = lat_cell(max_lat as f32);
    let mut out = Vec::new();
    if min_lon <= max_lon {
        let ix0 = lon_cell(min_lon as f32);
        let ix1 = lon_cell(max_lon as f32);
        for ix in ix0..=ix1 {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
    } else {
        let ix0 = lon_cell(min_lon as f32);
        let ix1 = lon_cell(max_lon as f32);
        let max_ix = lon_cell(180.0);
        for ix in ix0..=max_ix {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
        for ix in 0..=ix1 {
            for iy in iy0..=iy1 {
                out.push((ix, iy));
            }
        }
    }
    out
}

fn bbox_overlap(b: &[f32; 4], min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> bool {
    let overlaps = |a0: f64, a1: f64, b0: f64, b1: f64| a0 <= b1 && b0 <= a1;
    let lat_ok = overlaps(b[1] as f64, b[3] as f64, min_lat, max_lat);
    if !lat_ok {
        return false;
    }
    if min_lon <= max_lon {
        overlaps(b[0] as f64, b[2] as f64, min_lon, max_lon)
    } else {
        overlaps(b[0] as f64, b[2] as f64, min_lon, 180.0)
            || overlaps(b[0] as f64, b[2] as f64, -180.0, max_lon)
    }
}

fn view_lonlat_bounds(view: &ViewCtx) -> (f64, f64, f64, f64) {
    if view.mode.is_globe() {
        // 半球外接粗估：按 zoom 放大可视角（比原先略紧，减少背面误选）
        let half = (75.0 / view.zoom.max(2.0)).clamp(20.0, 85.0);
        let min_lat = (view.center_lat - half).max(-90.0);
        let max_lat = (view.center_lat + half).min(90.0);
        let cos_lat = view.center_lat.to_radians().cos().abs().max(0.25);
        let mut min_lon = view.center_lon - half / cos_lat;
        let mut max_lon = view.center_lon + half / cos_lat;
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
        if max_lon - min_lon > 300.0 {
            let half_w = (view.rect.width() as f64) / projection::world_size(view.zoom) * 180.0
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
    if zoom < 2.8 {
        8
    } else if zoom < 3.8 {
        4
    } else if zoom < 5.2 {
        2
    } else {
        1
    }
}

/// 线段抽稀步长：远距只画 1/N；中国层更稀（与 GEM 叠绘）。
fn seg_stride(zoom: f64, china: bool) -> usize {
    let base = if zoom < 2.5 {
        14
    } else if zoom < 3.2 {
        8
    } else if zoom < 4.0 {
        4
    } else if zoom < 5.5 {
        2
    } else {
        1
    };
    if china && zoom < 5.0 {
        (base * 2).min(20)
    } else {
        base
    }
}

fn min_diag_deg(zoom: f64) -> f32 {
    if zoom < 2.8 {
        1.1
    } else if zoom < 3.8 {
        0.55
    } else if zoom < 5.0 {
        0.22
    } else if zoom < 6.5 {
        0.08
    } else {
        0.0
    }
}

fn stroke_style(dark: bool, zoom: f64) -> (Color32, Color32, f32) {
    let w = if zoom < 4.0 {
        0.7_f32
    } else if zoom < 6.5 {
        0.95_f32
    } else {
        1.15_f32
    };
    if dark {
        (
            Color32::from_rgba_unmultiplied(0xC4, 0x6A, 0x4A, 160),
            Color32::from_rgba_unmultiplied(0xE0, 0x7A, 0x52, 200),
            w,
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(0x9A, 0x3E, 0x2A, 170),
            Color32::from_rgba_unmultiplied(0xB5, 0x45, 0x2E, 210),
            w,
        )
    }
}

fn paint_key(view: &ViewCtx, dark: bool, ppp: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    view.mode.hash(&mut h);
    // 量化：兼顾拖拽流畅与断层相对底图不错位（过粗会「粘滞」）
    ((view.center_lon * 100.0).round() as i64).hash(&mut h);
    ((view.center_lat * 100.0).round() as i64).hash(&mut h);
    ((view.zoom * 28.0).round() as i64).hash(&mut h);
    ((view.rect.width() * 0.5).round() as i32).hash(&mut h);
    ((view.rect.height() * 0.5).round() as i32).hash(&mut h);
    ((ppp * 4.0).round() as i32).hash(&mut h);
    dark.hash(&mut h);
    h.finish()
}

fn paint_layer(
    mesh: &mut Mesh,
    layer: &SegLayer,
    view: &ViewCtx,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    step: usize,
    stride: usize,
    min_diag: f32,
    width: f32,
    color: Color32,
    budget: usize,
) -> usize {
    if budget == 0 || layer.segs.is_empty() {
        return 0;
    }
    let step = step.max(1);
    let stride = stride.max(1);
    let candidates = layer.query_candidates(min_lon, min_lat, max_lon, max_lat);
    let mut painted = 0usize;
    // 长断层在远距抽稀时仍保留：对角线超过阈值的不按 stride 丢弃
    let keep_diag = (min_diag * 3.0).max(2.5);

    for idx in candidates {
        if painted >= budget {
            break;
        }
        let seg = &layer.segs[idx as usize];
        if !bbox_overlap(&seg.bbox, min_lon, min_lat, max_lon, max_lat) {
            continue;
        }
        if min_diag > 0.0 && seg.diag < min_diag {
            continue;
        }
        if stride > 1 && seg.diag < keep_diag && (idx as usize) % stride != 0 {
            continue;
        }

        if step == 1 {
            paint_line_mesh(mesh, &seg.line, view, width, color);
        } else {
            let mut thin = Vec::with_capacity(seg.line.len() / step + 2);
            for (i, p) in seg.line.iter().enumerate() {
                if i % step == 0 || i + 1 == seg.line.len() {
                    thin.push(*p);
                }
            }
            if thin.len() >= 2 {
                paint_line_mesh(mesh, &thin, view, width, color);
            }
        }
        painted += 1;
    }
    painted
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

fn paint_line_mesh(
    mesh: &mut Mesh,
    line: &[[f32; 2]],
    view: &ViewCtx,
    width: f32,
    color: Color32,
) {
    let mut pts = Vec::with_capacity(line.len());
    let mut last: Option<Pos2> = None;
    let max_jump = if view.mode.is_globe() {
        view.horizon_radius().unwrap_or(80.0) * 0.35
    } else {
        projection::world_size(view.zoom) as f32 * (80.0 / 360.0)
    };
    for p in line {
        let Some(sp) = view.project(p[0] as f64, p[1] as f64) else {
            stroke_poly(mesh, &mut pts, width, color);
            last = None;
            continue;
        };
        // 视口外略扩一点仍投影，靠 mesh 裁剪；过远跳点后断开
        if let Some(prev) = last {
            let dx = sp.x - prev.x;
            let dy = sp.y - prev.y;
            if dx * dx + dy * dy > max_jump * max_jump {
                stroke_poly(mesh, &mut pts, width, color);
            }
        }
        pts.push(sp);
        last = Some(sp);
    }
    stroke_poly(mesh, &mut pts, width, color);
}

/// 折线描边压入 Mesh（每段四边形），避免 PathShape 每帧镶嵌。
fn stroke_poly(mesh: &mut Mesh, pts: &mut Vec<Pos2>, width: f32, color: Color32) {
    if pts.len() < 2 {
        pts.clear();
        return;
    }
    let half = (width * 0.5).max(0.35);
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.4 {
            continue;
        }
        let nx = -dy / len * half;
        let ny = dx / len * half;
        let i0 = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(a.x + nx, a.y + ny), color);
        mesh.colored_vertex(Pos2::new(a.x - nx, a.y - ny), color);
        mesh.colored_vertex(Pos2::new(b.x + nx, b.y + ny), color);
        mesh.colored_vertex(Pos2::new(b.x - nx, b.y - ny), color);
        mesh.add_triangle(i0, i0 + 1, i0 + 2);
        mesh.add_triangle(i0 + 1, i0 + 3, i0 + 2);
    }
    pts.clear();
}
