//! 行政区矢量底图（暗色要石 / 亮色浅填）。
//!
//! 优先 `geodata/kanameishi/`（要石 TopoJSON / 展开 GeoJSON）；否则回退 Globalmap + 省市区划。
//! 加载在后台线程分两阶段（世界 → 区划），主线程 `poll` 合并。

use crate::projection::{self, ViewCtx};
use crate::topojson;
use egui::{Color32, Pos2, Rect, Stroke, Ui};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use tracing::{info, warn};

/// 海洋/缺数据底色（对齐 RhythmQuake / 要石 KA：#202124）
pub const OCEAN: Color32 = Color32::from_rgb(0x20, 0x21, 0x24);

/// 矢量陆地 / 边界色（随界面主题切换）。
#[derive(Debug, Clone, Copy)]
pub struct VectorStyle {
    pub land_fill: Color32,
    pub world_border: Color32,
    pub admin_border: Color32,
}

impl VectorStyle {
    pub fn dark() -> Self {
        Self {
            // 陆地灰 / 边界略亮，便于在黑海上分清区划
            land_fill: Color32::from_rgb(0x4A, 0x4A, 0x4E),
            world_border: Color32::from_rgb(0xC8, 0xC8, 0xCC),
            admin_border: Color32::from_rgb(0xA8, 0xA8, 0xB0),
        }
    }

    pub fn light() -> Self {
        Self {
            land_fill: Color32::from_rgb(0xE8, 0xEC, 0xF0),
            world_border: Color32::from_rgb(0x6B, 0x72, 0x80),
            admin_border: Color32::from_rgb(0x9C, 0xA3, 0xAF),
        }
    }

    pub fn for_theme(theme: &str) -> Self {
        if theme.eq_ignore_ascii_case("light") {
            Self::light()
        } else {
            Self::dark()
        }
    }
}

impl Default for VectorStyle {
    fn default() -> Self {
        Self::dark()
    }
}

/// 低于此缩放只画中日外轮廓；高于此才叠省界/都道府县（世界层本身不含中日）。
const ZOOM_REGIONAL: f64 = 4.5;
const ZOOM_REGIONAL_MED: f64 = 6.5;
const ZOOM_REGIONAL_FINE: f64 = 8.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasemapMode {
    Vector,
    Tiles,
    Both,
}

impl BasemapMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "tiles" | "tile" | "raster" => Self::Tiles,
            "both" | "overlay" => Self::Both,
            _ => Self::Vector,
        }
    }

    /// `auto`：亮色→地形瓦片；暗色→瓦片+矢量描边（陆地不再耳切填色，避免灰三角）。
    pub fn resolve(configured: &str, theme: &str) -> Self {
        match configured.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => {
                if theme.eq_ignore_ascii_case("light") {
                    Self::Tiles
                } else {
                    Self::Both
                }
            }
            other => Self::parse(other),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Tiles => "tiles",
            Self::Both => "both",
        }
    }

    pub fn show_vector(self) -> bool {
        matches!(self, Self::Vector | Self::Both)
    }

    pub fn show_tiles(self) -> bool {
        matches!(self, Self::Tiles | Self::Both)
    }
}

#[derive(Clone)]
struct RingSet {
    exterior: Vec<[f32; 2]>,
    holes: Vec<Vec<[f32; 2]>>,
    min_lon: f32,
    min_lat: f32,
    max_lon: f32,
    max_lat: f32,
    /// 沿环解缠后的经度跨度；>180° 表示绕地过半，禁止填色。
    lon_span: f32,
    /// 经纬度平面耳切三角剖分（外环+孔洞顶点序）；空表示不可填。
    tris: Vec<u32>,
}

impl RingSet {
    fn from_parts(exterior: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Option<Self> {
        if exterior.len() < 3 {
            return None;
        }
        let lon_span = unwrapped_lon_span(&exterior);
        let (min_lon, min_lat, max_lon, max_lat) = bbox_of(&exterior);
        let mut ring = Self {
            exterior,
            holes,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            lon_span,
            tris: Vec::new(),
        };
        if ring.may_fill() {
            ring.tris = triangulate_ring(&ring.exterior, &ring.holes);
        }
        Some(ring)
    }

    /// 绕地过半或跨日界的环只描边；其余用 earcut 预剖分后 Mesh 填色。
    fn may_fill(&self) -> bool {
        self.lon_span <= 180.0 + 1e-3 && !has_antimeridian_edge(&self.exterior)
    }
}

struct LayerLods {
    coarse: Vec<RingSet>,
    medium: Vec<RingSet>,
    fine: Vec<RingSet>,
}

impl LayerLods {
    fn from_fine(fine: Vec<RingSet>, coarse_tol: f64, medium_tol: f64) -> Self {
        let medium: Vec<RingSet> = fine
            .iter()
            .flat_map(|r| simplify_ringsets(r, medium_tol))
            .collect();
        let coarse: Vec<RingSet> = fine
            .iter()
            .flat_map(|r| simplify_ringsets(r, coarse_tol))
            .collect();
        Self {
            coarse,
            medium,
            fine,
        }
    }

    fn pick(&self, zoom: f64, z_med: f64, z_fine: f64) -> &Vec<RingSet> {
        if zoom < z_med {
            &self.coarse
        } else if zoom < z_fine {
            &self.medium
        } else {
            &self.fine
        }
    }
}

enum LoadPacket {
    World {
        lods: LayerLods,
        rings: usize,
        files: usize,
        skipped: usize,
    },
    Regional {
        lods: LayerLods,
        national: LayerLods,
        rings: usize,
        files: usize,
        skipped: usize,
    },
}

/// 矢量底图加载阶段（供 HUD 文案）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasemapLoadState {
    Empty,
    LoadingWorld,
    LoadingRegional,
    Ready,
    Failed,
}

/// 视口不变时复用投影后的 Shape，避免每帧全量投影抬高 CPU。
struct VectorPaintCache {
    key: u64,
    shapes: Vec<egui::Shape>,
}

pub struct VectorBasemap {
    world: LayerLods,
    /// 中日省界 / 都道府县（近距）
    regional: LayerLods,
    /// 中日外轮廓（远距；由区划边界溶解得到）
    national: LayerLods,
    world_ready: bool,
    regional_ready: bool,
    state: BasemapLoadState,
    status: String,
    gen: u64,
    rx: Option<Receiver<(u64, LoadPacket)>>,
    loaded_dir: Option<PathBuf>,
    paint_cache: Option<VectorPaintCache>,
}

impl Default for VectorBasemap {
    fn default() -> Self {
        Self {
            world: LayerLods {
                coarse: Vec::new(),
                medium: Vec::new(),
                fine: Vec::new(),
            },
            regional: LayerLods {
                coarse: Vec::new(),
                medium: Vec::new(),
                fine: Vec::new(),
            },
            national: LayerLods {
                coarse: Vec::new(),
                medium: Vec::new(),
                fine: Vec::new(),
            },
            world_ready: false,
            regional_ready: false,
            state: BasemapLoadState::Empty,
            status: String::new(),
            gen: 0,
            rx: None,
            loaded_dir: None,
            paint_cache: None,
        }
    }
}

impl VectorBasemap {
    pub fn is_loaded(&self) -> bool {
        self.world_ready || self.regional_ready
    }

    pub fn load_state(&self) -> BasemapLoadState {
        self.state
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    /// 后台分两阶段加载：先全球轮廓（可立即绘制），再中日区划。
    /// 同一目录且已就绪时跳过；目录变更或失败后可再次调用。
    pub fn load_dir(&mut self, dir: impl AsRef<Path>) {
        let dir = dir.as_ref().to_path_buf();
        if !dir.is_dir() {
            warn!(path = %dir.display(), "geodata 目录不存在，矢量底图未加载");
            self.state = BasemapLoadState::Failed;
            self.status = format!("geodata 不存在：{}", dir.display());
            return;
        }
        if self.loaded_dir.as_ref() == Some(&dir)
            && self.state == BasemapLoadState::Ready
            && self.world_ready
        {
            return;
        }
        if self.loaded_dir.as_ref() == Some(&dir)
            && matches!(
                self.state,
                BasemapLoadState::LoadingWorld | BasemapLoadState::LoadingRegional
            )
        {
            return;
        }

        self.gen = self.gen.wrapping_add(1);
        let gen = self.gen;
        self.loaded_dir = Some(dir.clone());
        // 保留已加载层，避免重载时闪成纯海洋黑底；新包到达后再替换
        if self.world_ready || self.regional_ready {
            self.state = BasemapLoadState::LoadingRegional;
            self.status = "正在更新底图…".into();
        } else {
            self.state = BasemapLoadState::LoadingWorld;
            self.status = "正在加载世界底图…".into();
        }

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        let _ = thread::Builder::new()
            .name("eewview-geodata".into())
            .spawn(move || {
                let (world_fine, w_files, w_skip) = load_world_rings(&dir);
                let world_n = world_fine.len();
                let world_lods = LayerLods::from_fine(world_fine, 0.35, 0.12);
                if tx
                    .send((
                        gen,
                        LoadPacket::World {
                            lods: world_lods,
                            rings: world_n,
                            files: w_files,
                            skipped: w_skip,
                        },
                    ))
                    .is_err()
                {
                    return;
                }

                let (mut regional_fine, r_files, r_skip) = load_regional_rings(&dir);
                // 藏南：不另画凸包；要石 cn.province 西藏面已含主张范围，区划/国界叠在世界层之上即可
                let diaoyu = builtin_diaoyu_patches();
                let mut dissolve_src = regional_fine.clone();
                dissolve_src.extend(diaoyu.iter().cloned());
                let national_fine = dissolve_national_outlines(&dissolve_src);
                regional_fine.extend(diaoyu);
                let regional_n = regional_fine.len();
                let national_lods = LayerLods::from_fine(national_fine, 0.25, 0.08);
                let regional_lods = LayerLods::from_fine(regional_fine, 0.05, 0.012);
                let _ = tx.send((
                    gen,
                    LoadPacket::Regional {
                        lods: regional_lods,
                        national: national_lods,
                        rings: regional_n,
                        files: r_files,
                        skipped: r_skip,
                    },
                ));
            });
    }

    /// 主线程每帧拉取后台结果；有更新时返回 true。
    pub fn poll(&mut self) -> bool {
        let Some(rx) = self.rx.take() else {
            return false;
        };
        let mut changed = false;
        let mut keep = true;
        while let Ok((gen, packet)) = rx.try_recv() {
            if gen != self.gen {
                continue;
            }
            changed = true;
            self.paint_cache = None;
            match packet {
                LoadPacket::World {
                    lods,
                    rings,
                    files,
                    skipped,
                } => {
                    self.world = lods;
                    self.world_ready = !self.world.fine.is_empty();
                    self.state = BasemapLoadState::LoadingRegional;
                    self.status = if self.world_ready {
                        format!("世界底图就绪（{rings} 环），正在加载区划…")
                    } else {
                        "世界底图为空，正在加载区划…".into()
                    };
                    info!(
                        rings,
                        files,
                        skipped,
                        "vector world layer ready"
                    );
                }
                LoadPacket::Regional {
                    lods,
                    national,
                    rings,
                    files,
                    skipped,
                } => {
                    self.regional = lods;
                    self.national = national;
                    self.regional_ready =
                        !self.regional.fine.is_empty() || !self.national.fine.is_empty();
                    self.state = if self.world_ready || self.regional_ready {
                        BasemapLoadState::Ready
                    } else {
                        BasemapLoadState::Failed
                    };
                    self.status = if self.state == BasemapLoadState::Ready {
                        String::new()
                    } else {
                        "矢量底图加载失败（无可用几何）".into()
                    };
                    info!(
                        rings,
                        files,
                        skipped,
                        world = self.world.fine.len(),
                        regional = self.regional.fine.len(),
                        national = self.national.fine.len(),
                        "vector basemap fully loaded"
                    );
                    keep = false;
                }
            }
        }
        if keep {
            self.rx = Some(rx);
        }
        changed
    }

    pub fn paint(
        &mut self,
        ui: &mut Ui,
        view: &ViewCtx,
        fill_land: bool,
        style: VectorStyle,
    ) {
        if !self.world_ready && !self.regional_ready {
            return;
        }

        let rect = view.rect;
        let zoom = view.zoom;
        let painter = ui.painter_at(rect);
        let cache_key = paint_cache_key(view, fill_land, style, self.gen);
        if let Some(c) = self.paint_cache.as_ref() {
            if c.key == cache_key {
                painter.extend(c.shapes.clone());
                return;
            }
        }
        let mut shapes = Vec::with_capacity(512);

        if view.mode.is_globe() {
            let max_jump_px = view
                .horizon_radius()
                .map(|r| r * 0.35)
                .unwrap_or(80.0)
                .max(40.0);
            // 球体：半球可见，不做横向副本
            let (min_lon, min_lat, max_lon, max_lat) = (-180.0, -90.0, 180.0, 90.0);
            if !self.world.fine.is_empty() {
                let rings = self.world.pick(zoom, 4.0, 6.0);
                let min_seg = if zoom < 4.0 { 2.2_f32 } else { 1.4_f32 };
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    Stroke::new(0.9_f32, style.world_border),
                    fill_land,
                    false,
                    style.land_fill,
                    0.0,
                );
            }
            // 远距只画中日外轮廓；近距外轮廓仍保留，再叠省界
            if !self.national.fine.is_empty() {
                let rings = self.national.pick(zoom, 3.0, 4.0);
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    2.2,
                    max_jump_px,
                    Stroke::new(0.95_f32, style.world_border),
                    fill_land,
                    false,
                    style.land_fill,
                    0.0,
                );
            }
            if zoom >= ZOOM_REGIONAL && !self.regional.fine.is_empty() {
                let rings = self
                    .regional
                    .pick(zoom, ZOOM_REGIONAL_MED, ZOOM_REGIONAL_FINE);
                let min_seg = if zoom < ZOOM_REGIONAL_MED {
                    2.0_f32
                } else if zoom < ZOOM_REGIONAL_FINE {
                    1.4_f32
                } else {
                    1.0_f32
                };
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    Stroke::new(
                        if zoom < ZOOM_REGIONAL_MED {
                            0.7_f32
                        } else {
                            0.95_f32
                        },
                        style.admin_border,
                    ),
                    fill_land,
                    zoom >= ZOOM_REGIONAL_FINE,
                    style.land_fill,
                    0.0,
                );
            }
            self.paint_cache = Some(VectorPaintCache {
                key: cache_key,
                shapes: shapes.clone(),
            });
            painter.extend(shapes);
            return;
        }

        let world_px = projection::world_size(zoom) as f32;
        let full_wrap = rect.width() >= world_px * 0.92;
        let (min_lon, min_lat, max_lon, max_lat) = if full_wrap {
            (-180.0, -85.0, 180.0, 85.0)
        } else {
            viewport_bounds(view.center_lon, view.center_lat, zoom, rect)
        };
        let max_jump_px = world_px * (80.0 / 360.0);
        let half_copies = ((rect.width() * 0.5 / world_px.max(1.0)).ceil() as i32 + 1)
            .clamp(1, 6);

        for k in -half_copies..=half_copies {
            let lon_shift = f64::from(k) * 360.0;

            if !self.world.fine.is_empty() {
                let rings = self.world.pick(zoom, 4.0, 6.0);
                let min_seg = if zoom < 4.0 { 2.2_f32 } else { 1.4_f32 };
                let stroke = Stroke::new(0.9_f32, style.world_border);
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon + lon_shift,
                    min_lat,
                    max_lon + lon_shift,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    stroke,
                    fill_land,
                    false,
                    style.land_fill,
                    lon_shift,
                );
            }

            if !self.national.fine.is_empty() {
                let rings = self.national.pick(zoom, 3.0, 4.0);
                let stroke = Stroke::new(0.95_f32, style.world_border);
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon + lon_shift,
                    min_lat,
                    max_lon + lon_shift,
                    max_lat,
                    2.2,
                    max_jump_px,
                    stroke,
                    fill_land,
                    false,
                    style.land_fill,
                    lon_shift,
                );
            }
            if zoom >= ZOOM_REGIONAL && !self.regional.fine.is_empty() {
                let rings = self
                    .regional
                    .pick(zoom, ZOOM_REGIONAL_MED, ZOOM_REGIONAL_FINE);
                let min_seg = if zoom < ZOOM_REGIONAL_MED {
                    2.0_f32
                } else if zoom < ZOOM_REGIONAL_FINE {
                    1.4_f32
                } else {
                    1.0_f32
                };
                let stroke = Stroke::new(
                    if zoom < ZOOM_REGIONAL_MED {
                        0.7_f32
                    } else {
                        0.95_f32
                    },
                    style.admin_border,
                );
                paint_rings(
                    &mut shapes,
                    rings,
                    view,
                    min_lon + lon_shift,
                    min_lat,
                    max_lon + lon_shift,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    stroke,
                    fill_land,
                    zoom >= ZOOM_REGIONAL_FINE,
                    style.land_fill,
                    lon_shift,
                );
            }
        }

        self.paint_cache = Some(VectorPaintCache {
            key: cache_key,
            shapes: shapes.clone(),
        });
        painter.extend(shapes);
    }
}

fn paint_cache_key(view: &ViewCtx, fill_land: bool, style: VectorStyle, gen: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    gen.hash(&mut h);
    view.center_lon.to_bits().hash(&mut h);
    view.center_lat.to_bits().hash(&mut h);
    view.zoom.to_bits().hash(&mut h);
    (view.rect.width() as i32).hash(&mut h);
    (view.rect.height() as i32).hash(&mut h);
    (view.rect.min.x as i32).hash(&mut h);
    (view.rect.min.y as i32).hash(&mut h);
    fill_land.hash(&mut h);
    style.land_fill.to_array().hash(&mut h);
    style.world_border.to_array().hash(&mut h);
    style.admin_border.to_array().hash(&mut h);
    std::mem::discriminant(&view.mode).hash(&mut h);
    h.finish()
}

fn paint_rings(
    shapes: &mut Vec<egui::Shape>,
    rings: &[RingSet],
    view: &ViewCtx,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    min_seg_px: f32,
    max_jump_px: f32,
    stroke: Stroke,
    fill: bool,
    draw_holes: bool,
    land_fill: Color32,
    lon_shift: f64,
) {
    // 先填后描。灰底在加载时预剖分，绘制时只投影；球体按三角形常驻裁剪，
    // 不因整环有一点落在地平线后就整片丢弃。
    if fill {
        let mut land = egui::epaint::Mesh::default();
        for ring in rings {
            if ring.tris.is_empty() {
                continue;
            }
            if view.mode.is_globe() {
                append_globe_fill_tris(&mut land, ring, view, land_fill);
                continue;
            }
            if !ring_visible(
                ring,
                min_lon - lon_shift,
                min_lat,
                max_lon - lon_shift,
                max_lat,
            ) {
                continue;
            }
            if let Some(verts) = project_fill_vertices(ring, view, lon_shift) {
                let base = land.vertices.len() as u32;
                land.reserve_vertices(verts.len());
                land.reserve_triangles(ring.tris.len() / 3);
                for p in verts {
                    land.colored_vertex(p, land_fill);
                }
                for tri in ring.tris.chunks_exact(3) {
                    land.add_triangle(base + tri[0], base + tri[1], base + tri[2]);
                }
            }
        }
        if !land.indices.is_empty() {
            shapes.push(land.into());
        }
    }

    for ring in rings {
        // 可见性按未偏移地理范围；副本仅改投影经度
        if !ring_visible(
            ring,
            min_lon - lon_shift,
            min_lat,
            max_lon - lon_shift,
            max_lat,
        ) {
            continue;
        }

        let segments = project_segments(
            &ring.exterior,
            view,
            min_seg_px,
            max_jump_px,
            lon_shift,
        );
        for pts in segments {
            if pts.len() < 2 {
                continue;
            }
            shapes.push(
                egui::epaint::PathShape {
                    points: pts,
                    closed: false,
                    fill: Color32::TRANSPARENT,
                    stroke: egui::epaint::PathStroke::new(stroke.width, stroke.color),
                }
                .into(),
            );
        }

        if draw_holes {
            for hole in &ring.holes {
                let hsegs = project_segments(
                    hole,
                    view,
                    min_seg_px,
                    max_jump_px,
                    lon_shift,
                );
                for hpts in hsegs {
                    if hpts.len() < 2 {
                        continue;
                    }
                    shapes.push(
                        egui::epaint::PathShape {
                            points: hpts,
                            closed: false,
                            fill: Color32::TRANSPARENT,
                            stroke: egui::epaint::PathStroke::new(
                                stroke.width * 0.7,
                                stroke.color.gamma_multiply(0.75),
                            ),
                        }
                        .into(),
                    );
                }
            }
        }
    }
}

/// 经纬度耳切；顶点序 = 外环 + 各孔洞，与 `project_fill_vertices` 一致。
fn triangulate_ring(exterior: &[[f32; 2]], holes: &[Vec<[f32; 2]>]) -> Vec<u32> {
    let mut flat: Vec<f64> = Vec::with_capacity((exterior.len()
        + holes.iter().map(|h| h.len()).sum::<usize>())
        * 2);
    let mut hole_indices: Vec<usize> = Vec::with_capacity(holes.len());
    for p in exterior {
        flat.push(p[0] as f64);
        flat.push(p[1] as f64);
    }
    for hole in holes {
        if hole.len() < 3 {
            continue;
        }
        hole_indices.push(flat.len() / 2);
        for p in hole {
            flat.push(p[0] as f64);
            flat.push(p[1] as f64);
        }
    }
    match earcutr::earcut(&flat, &hole_indices, 2) {
        Ok(idx) => idx.into_iter().map(|i| i as u32).collect(),
        Err(_) => Vec::new(),
    }
}

/// 球体填色：优先用地平线内顶点；至少两顶点可见时用 raw 补第三点，减少美/俄球缘缺块。
fn append_globe_fill_tris(
    land: &mut egui::epaint::Mesh,
    ring: &RingSet,
    view: &ViewCtx,
    land_fill: Color32,
) {
    let n = ring.exterior.len() + ring.holes.iter().map(|h| h.len()).sum::<usize>();
    let mut lons: Vec<f64> = Vec::with_capacity(n);
    let mut lats: Vec<f64> = Vec::with_capacity(n);
    let mut screen: Vec<Option<Pos2>> = Vec::with_capacity(n);
    for p in &ring.exterior {
        let lon = p[0] as f64;
        let lat = p[1] as f64;
        lons.push(lon);
        lats.push(lat);
        screen.push(view.project_fill(lon, lat));
    }
    for hole in &ring.holes {
        if hole.len() < 3 {
            continue;
        }
        for p in hole {
            let lon = p[0] as f64;
            let lat = p[1] as f64;
            lons.push(lon);
            lats.push(lat);
            screen.push(view.project_fill(lon, lat));
        }
    }
    if screen.len() < 3 {
        return;
    }

    land.reserve_triangles(ring.tris.len() / 3);
    for tri in ring.tris.chunks_exact(3) {
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        let mut a = screen.get(i0).copied().flatten();
        let mut b = screen.get(i1).copied().flatten();
        let mut c = screen.get(i2).copied().flatten();
        let visible = a.is_some() as u8 + b.is_some() as u8 + c.is_some() as u8;
        if visible < 2 {
            continue;
        }
        if a.is_none() {
            a = view.project_fill_raw(lons[i0], lats[i0]);
        }
        if b.is_none() {
            b = view.project_fill_raw(lons[i1], lats[i1]);
        }
        if c.is_none() {
            c = view.project_fill_raw(lons[i2], lats[i2]);
        }
        let (Some(a), Some(b), Some(c)) = (a, b, c) else {
            continue;
        };
        // 圆形遮罩外的 raw 补点丢弃，避免陆地画进球外四角
        if let Some(hr) = view.horizon_radius() {
            let cxy = view.rect.center();
            let lim2 = (hr * 1.04) * (hr * 1.04);
            if (a - cxy).length_sq() > lim2
                || (b - cxy).length_sq() > lim2
                || (c - cxy).length_sq() > lim2
            {
                continue;
            }
        }
        let base = land.vertices.len() as u32;
        land.colored_vertex(a, land_fill);
        land.colored_vertex(b, land_fill);
        land.colored_vertex(c, land_fill);
        land.add_triangle(base, base + 1, base + 2);
    }
}

/// 平面填色：整环连续解缠 + `lon_shift`，避免按点 wrap 在视口边缘撕环丢填色。
fn project_fill_vertices(
    ring: &RingSet,
    view: &ViewCtx,
    lon_shift: f64,
) -> Option<Vec<Pos2>> {
    let mut out: Vec<Pos2> =
        Vec::with_capacity(ring.exterior.len() + ring.holes.iter().map(|h| h.len()).sum::<usize>());

    let push_ring = |pts: &[[f32; 2]], out: &mut Vec<Pos2>| -> bool {
        let Some(projected) = project_ring_points_fill(pts, view, lon_shift) else {
            return false;
        };
        if projected.len() != pts.len() {
            return false;
        }
        out.extend(projected);
        true
    };

    if !push_ring(&ring.exterior, &mut out) {
        return None;
    }
    for hole in &ring.holes {
        if hole.len() < 3 {
            continue;
        }
        if !push_ring(hole, &mut out) {
            return None;
        }
    }
    if out.len() < 3 {
        return None;
    }
    Some(out)
}

fn project_ring_points_fill(
    ring: &[[f32; 2]],
    view: &ViewCtx,
    lon_shift: f64,
) -> Option<Vec<Pos2>> {
    if ring.len() < 3 {
        return None;
    }

    // 先解缠成连续经度轨迹，再整体加世界副本偏移；禁止对单点 wrap_lon_near，
    // 否则大块陆地在中心移到环「背面」时会被撕成断环并丢弃填色。
    let unwrapped = unwrap_ring(ring);
    if unwrapped.len() != ring.len() {
        return None;
    }
    let zoom = view.zoom;
    let rect = view.rect;
    let (cx, cy) = projection::lonlat_to_world(view.center_lon, view.center_lat, zoom);
    let mut out = Vec::with_capacity(unwrapped.len());
    for p in &unwrapped {
        let lon = p[0] as f64 + lon_shift;
        let lat = p[1] as f64;
        let (wx, wy) = projection::lonlat_to_world(lon, lat, zoom);
        out.push(rect.center() + egui::Vec2::new((wx - cx) as f32, (wy - cy) as f32));
    }
    Some(out)
}

fn ring_visible(ring: &RingSet, min_lon: f64, min_lat: f64, max_lon: f64, max_lat: f64) -> bool {
    if (ring.max_lat as f64) < min_lat || (ring.min_lat as f64) > max_lat {
        return false;
    }
    lon_ranges_overlap(
        ring.min_lon as f64,
        ring.max_lon as f64,
        min_lon,
        max_lon,
    )
}

fn lon_ranges_overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    for shift in [-360.0, 0.0, 360.0] {
        let lo = a0 + shift;
        let hi = a1 + shift;
        if hi >= b0 && lo <= b1 {
            return true;
        }
    }
    false
}


fn find_globalmap(dir: &Path) -> Option<std::path::PathBuf> {
    const CANDIDATES: &[&str] = &[
        "Globalmap.geo.json",
        "Globalmap.geojson",
        "globalmap.geo.json",
        "globalmap.geojson",
        "GlobalMap.geo.json",
    ];
    for name in CANDIDATES {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn kanameishi_dir(dir: &Path) -> PathBuf {
    dir.join("kanameishi")
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 全球层：优先根目录精细 GeoJSON → 要石 Topo → Globalmap + `world/` 补丁。
fn load_world_rings(dir: &Path) -> (Vec<RingSet>, usize, usize) {
    let mut world_fine = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;

    // 根目录精细全球轮廓（与要石同名拓扑，展开更密）
    if let Some(world) = first_existing(
        dir,
        &[
            "medium.global.modified.geo.json",
            "medium.global.modified.geojson",
            "medium.global.geo.json",
        ],
    ) {
        files += 1;
        match load_any_map(&world) {
            Ok(rings) => {
                info!(
                    path = %world.display(),
                    rings = rings.len(),
                    "loaded fine world basemap GeoJSON"
                );
                world_fine.extend(rings);
                append_cn_boundary_patches(dir, &mut world_fine, &mut files, &mut skipped);
                return (world_fine, files, skipped);
            }
            Err(e) => {
                skipped += 1;
                warn!(
                    path = %world.display(),
                    error = %e,
                    "精细世界底图加载失败，回退要石"
                );
            }
        }
    }

    let kdir = kanameishi_dir(dir);
    if let Some(world) = first_existing(
        &kdir,
        &[
            "medium.global.modified.topo.json",
            "world.geo.json",
            "world.geojson",
        ],
    ) {
        files += 1;
        match load_any_map(&world) {
            Ok(rings) => {
                info!(
                    path = %world.display(),
                    rings = rings.len(),
                    "loaded kanameishi world basemap"
                );
                world_fine.extend(rings);
                // 要石世界层不含中日外轮廓；钓鱼岛点状叠盖（藏南靠中国省界上层叠盖）
                append_cn_boundary_patches(dir, &mut world_fine, &mut files, &mut skipped);
                return (world_fine, files, skipped);
            }
            Err(e) => {
                skipped += 1;
                warn!(path = %world.display(), error = %e, "要石世界底图加载失败，回退 Globalmap");
            }
        }
    }

    let globalmap = find_globalmap(dir);
    let use_globalmap = globalmap.is_some();
    if let Some(ref gm) = globalmap {
        files += 1;
        match load_file(gm) {
            Ok(rings) => {
                info!(
                    path = %gm.display(),
                    rings = rings.len(),
                    "loaded Globalmap as world basemap"
                );
                world_fine.extend(rings);
            }
            Err(e) => {
                skipped += 1;
                warn!(path = %gm.display(), error = %e, "Globalmap 加载失败");
            }
        }
    }

    let world_dir = dir.join("world");
    let world_entries = if world_dir.is_dir() {
        walk_geojson(&world_dir)
    } else {
        Vec::new()
    };

    for entry in world_entries {
        if globalmap.as_ref().is_some_and(|g| g == &entry) {
            continue;
        }
        // 补丁文件不加载（藏南碎块已废弃；钓鱼岛用内置点状）
        if is_cn_boundary_patch(&entry) {
            continue;
        }
        if use_globalmap {
            continue;
        }
        files += 1;
        match load_file(&entry) {
            Ok(rings) => world_fine.extend(rings),
            Err(e) => {
                skipped += 1;
                warn!(path = %entry.display(), error = %e, "跳过 GeoJSON");
            }
        }
    }

    world_fine.extend(builtin_diaoyu_patches());

    (world_fine, files, skipped)
}


/// 世界层只叠钓鱼岛（点状）。藏南不另描边，由中国省界叠盖。
fn append_cn_boundary_patches(
    dir: &Path,
    world_fine: &mut Vec<RingSet>,
    files: &mut usize,
    skipped: &mut usize,
) {
    let _ = dir;
    let diaoyu = builtin_diaoyu_patches();
    if diaoyu.is_empty() {
        *skipped += 1;
        return;
    }
    *files += 1;
    world_fine.extend(diaoyu);
}

/// 区划层：优先要石中/日省界；否则扫中国地市 + 日本都道府县。
fn load_regional_rings(dir: &Path) -> (Vec<RingSet>, usize, usize) {
    let mut regional_fine = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;

    let kdir = kanameishi_dir(dir);
    let cn = first_existing(
        &kdir,
        &[
            "cn.province.topo.json",
            "cn_province.geo.json",
            "cn_province.geojson",
        ],
    );
    let jp = first_existing(
        &kdir,
        &["jp.pref.topo.json", "jp_pref.geo.json", "jp_pref.geojson"],
    );
    if cn.is_some() || jp.is_some() {
        for path in [cn, jp].into_iter().flatten() {
            files += 1;
            match load_any_map(&path) {
                Ok(rings) => {
                    info!(
                        path = %path.display(),
                        rings = rings.len(),
                        "loaded kanameishi regional basemap"
                    );
                    regional_fine.extend(rings);
                }
                Err(e) => {
                    skipped += 1;
                    warn!(path = %path.display(), error = %e, "要石区划加载失败");
                }
            }
        }
        return (regional_fine, files, skipped);
    }

    let globalmap = find_globalmap(dir);
    for entry in walk_geojson_skip_world(dir) {
        if globalmap.as_ref().is_some_and(|g| g == &entry) {
            continue;
        }
        if is_cn_boundary_patch(&entry) {
            continue;
        }
        if entry
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("kanameishi"))
        {
            continue;
        }
        files += 1;
        match load_file(&entry) {
            Ok(rings) => regional_fine.extend(rings),
            Err(e) => {
                skipped += 1;
                warn!(path = %entry.display(), error = %e, "跳过 GeoJSON");
            }
        }
    }

    (regional_fine, files, skipped)
}

/// 从省界/都道府县溶解出国家外轮廓：只保留出现一次的边（内部省界成对抵消）。
fn dissolve_national_outlines(regions: &[RingSet]) -> Vec<RingSet> {
    if regions.is_empty() {
        return Vec::new();
    }

    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    struct QPt(i32, i32);

    fn qpt(p: [f32; 2]) -> QPt {
        QPt(
            (p[0] as f64 * 2000.0).round() as i32,
            (p[1] as f64 * 2000.0).round() as i32,
        )
    }

    // edge_key -> (count, representative endpoints in lon/lat)
    let mut edge_count: HashMap<(QPt, QPt), u32> = HashMap::new();
    let mut edge_geom: HashMap<(QPt, QPt), ([f32; 2], [f32; 2])> = HashMap::new();

    for ring in regions {
        let pts = &ring.exterior;
        if pts.len() < 3 {
            continue;
        }
        let n = pts.len();
        for i in 0..n {
            let a = pts[i];
            let b = pts[(i + 1) % n];
            let qa = qpt(a);
            let qb = qpt(b);
            if qa == qb {
                continue;
            }
            let key = if qa <= qb { (qa, qb) } else { (qb, qa) };
            *edge_count.entry(key).or_insert(0) += 1;
            edge_geom.entry(key).or_insert((a, b));
        }
    }

    // 邻接表：只保留 count==1 的外边界边
    let mut adj: HashMap<QPt, Vec<(QPt, [f32; 2], [f32; 2])>> = HashMap::new();
    for (key, &cnt) in &edge_count {
        if cnt != 1 {
            continue;
        }
        let (a, b) = edge_geom[key];
        let qa = key.0;
        let qb = key.1;
        // 按量化点方向存实际坐标
        let (a_pt, b_pt) = if qpt(a) == qa { (a, b) } else { (b, a) };
        adj.entry(qa).or_default().push((qb, a_pt, b_pt));
        adj.entry(qb).or_default().push((qa, b_pt, a_pt));
    }

    let mut used: HashMap<(QPt, QPt), bool> = HashMap::new();
    let mut outlines: Vec<RingSet> = Vec::new();

    let starts: Vec<QPt> = adj.keys().copied().collect();
    for start in starts {
        // 找一条未用边
        let Some(neigh) = adj.get(&start) else {
            continue;
        };
        let mut begin = None;
        for (to, _, _) in neigh {
            let e = if start <= *to {
                (start, *to)
            } else {
                (*to, start)
            };
            if !used.get(&e).copied().unwrap_or(false) {
                begin = Some(*to);
                break;
            }
        }
        let Some(first_to) = begin else {
            continue;
        };

        let mut ring_pts: Vec<[f32; 2]> = Vec::new();
        let mut cur = start;
        let mut next = first_to;
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 200_000 {
                break;
            }
            let e = if cur <= next { (cur, next) } else { (next, cur) };
            if used.insert(e, true).is_some() {
                break;
            }
            // 取 cur→next 的实际坐标
            let mut from_lonlat = None;
            let mut to_lonlat = None;
            if let Some(list) = adj.get(&cur) {
                for (to, a, b) in list {
                    if *to == next {
                        from_lonlat = Some(*a);
                        to_lonlat = Some(*b);
                        break;
                    }
                }
            }
            if let Some(a) = from_lonlat {
                if ring_pts.last().map(|p| *p != a).unwrap_or(true) {
                    ring_pts.push(a);
                }
            }
            if let Some(b) = to_lonlat {
                ring_pts.push(b);
            }

            if next == start {
                break;
            }
            // 从 next 找另一条未用边
            let Some(list) = adj.get(&next) else {
                break;
            };
            let mut found = None;
            for (to, _, _) in list {
                let e2 = if next <= *to {
                    (next, *to)
                } else {
                    (*to, next)
                };
                if !used.get(&e2).copied().unwrap_or(false) {
                    found = Some(*to);
                    break;
                }
            }
            let Some(n2) = found else {
                break;
            };
            cur = next;
            next = n2;
        }

        if ring_pts.len() >= 3 {
            // 去闭合重复点
            if ring_pts.len() >= 2 {
                let a = ring_pts[0];
                let b = *ring_pts.last().unwrap();
                if (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5 {
                    ring_pts.pop();
                }
            }
            outlines.extend(ringsets_from_polygon(ring_pts, Vec::new()));
        }
    }

    info!(rings = outlines.len(), "dissolved national outlines from regional");
    outlines
}

fn load_any_map(path: &Path) -> Result<Vec<RingSet>, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.ends_with(".topo.json") || name.ends_with(".topojson") {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let polys = topojson::load_region_polygons(&text)?;
        let mut out = Vec::with_capacity(polys.len());
        for (exterior, holes) in polys {
            out.extend(ringsets_from_polygon(exterior, holes));
        }
        return Ok(out);
    }
    load_file(path)
}

fn is_cn_boundary_patch(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| {
            let l = n.to_ascii_lowercase();
            l.contains("cn_boundary_patch") || l.contains("cn_boundary_patches")
        })
}

fn walk_geojson(dir: &Path) -> Vec<std::path::PathBuf> {
    walk_geojson_inner(dir, false)
}

fn walk_geojson_skip_world(dir: &Path) -> Vec<std::path::PathBuf> {
    walk_geojson_inner(dir, true)
}

fn walk_geojson_inner(dir: &Path, skip_world: bool) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_dir() {
                if skip_world
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.eq_ignore_ascii_case("world"))
                {
                    continue;
                }
                stack.push(p);
            } else if is_geojson_path(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn is_geojson_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let l = name.to_ascii_lowercase();
    l.ends_with(".geo.json")
        || l.ends_with(".geojson")
        || (l.ends_with(".json") && !l.ends_with(".geo.json"))
}

/// 钓鱼岛（点状，可描边）。
fn builtin_diaoyu_patches() -> Vec<RingSet> {
    let diaoyu: &[&[[f32; 2]]] = &[
        &[
            [123.45, 25.73],
            [123.50, 25.73],
            [123.50, 25.76],
            [123.45, 25.76],
            [123.45, 25.73],
        ],
        &[
            [123.67, 25.92],
            [123.70, 25.92],
            [123.70, 25.94],
            [123.67, 25.94],
            [123.67, 25.92],
        ],
        &[
            [124.55, 25.91],
            [124.57, 25.91],
            [124.57, 25.93],
            [124.55, 25.93],
            [124.55, 25.91],
        ],
    ];
    let mut out = Vec::new();
    for island in diaoyu {
        if let Some(r) = ringset_from_lonlat(island) {
            out.push(r);
        }
    }
    out
}

fn ringset_from_lonlat(pts: &[[f32; 2]]) -> Option<RingSet> {
    if pts.len() < 3 {
        return None;
    }
    let mut exterior: Vec<[f32; 2]> = pts.to_vec();
    if exterior.len() >= 2 {
        let a = exterior[0];
        let b = exterior[exterior.len() - 1];
        if (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6 {
            exterior.pop();
        }
    }
    ringsets_from_polygon(exterior, Vec::new()).into_iter().next()
}

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    geometry: Option<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    geom_type: String,
    coordinates: serde_json::Value,
}

fn load_file(path: &Path) -> Result<Vec<RingSet>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fc: FeatureCollection = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for feat in fc.features {
        let Some(geom) = feat.geometry else {
            continue;
        };
        match geom.geom_type.as_str() {
            "Polygon" => out.extend(polygon_ringsets(&geom.coordinates)),
            "MultiPolygon" => {
                let Some(arr) = geom.coordinates.as_array() else {
                    continue;
                };
                for poly in arr {
                    out.extend(polygon_ringsets(poly));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn polygon_ringsets(coords: &serde_json::Value) -> Vec<RingSet> {
    let Some(rings) = coords.as_array() else {
        return Vec::new();
    };
    if rings.is_empty() {
        return Vec::new();
    }
    let Some(exterior) = parse_ring(&rings[0]) else {
        return Vec::new();
    };
    if exterior.len() < 3 {
        return Vec::new();
    }
    let mut holes = Vec::new();
    for hole in rings.iter().skip(1) {
        if let Some(h) = parse_ring(hole) {
            if h.len() >= 3 {
                holes.push(h);
            }
        }
    }
    ringsets_from_polygon(exterior, holes)
}

/// 将多边形沿日界线切开，得到可安全投影/填色的若干环。
fn ringsets_from_polygon(exterior: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Vec<RingSet> {
    let ext_parts = split_closed_ring(&exterior);
    if ext_parts.is_empty() {
        return Vec::new();
    }
    let hole_parts: Vec<Vec<[f32; 2]>> = holes
        .iter()
        .flat_map(|h| split_closed_ring(h))
        .filter(|h| h.len() >= 3 && unwrapped_lon_span(h) <= 180.0 + 1e-3)
        .collect();

    if ext_parts.len() == 1 {
        return RingSet::from_parts(ext_parts.into_iter().next().unwrap(), hole_parts)
            .into_iter()
            .collect();
    }

    // 多片外环：孔洞不做归属（避免错挂），各自独立
    ext_parts
        .into_iter()
        .filter_map(|e| RingSet::from_parts(e, Vec::new()))
        .collect()
}

fn parse_ring(v: &serde_json::Value) -> Option<Vec<[f32; 2]>> {
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
    if pts.len() >= 2 {
        let a = pts[0];
        let b = pts[pts.len() - 1];
        if (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6 {
            pts.pop();
        }
    }
    Some(pts)
}

fn bbox_of(pts: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut min_lon = f32::MAX;
    let mut min_lat = f32::MAX;
    let mut max_lon = f32::MIN;
    let mut max_lat = f32::MIN;
    for p in pts {
        min_lon = min_lon.min(p[0]);
        min_lat = min_lat.min(p[1]);
        max_lon = max_lon.max(p[0]);
        max_lat = max_lat.max(p[1]);
    }
    (min_lon, min_lat, max_lon, max_lat)
}

fn simplify_ringsets(src: &RingSet, tol_deg: f64) -> Vec<RingSet> {
    let exterior = douglas_peucker(&src.exterior, tol_deg);
    let holes: Vec<Vec<[f32; 2]>> = src
        .holes
        .iter()
        .map(|h| douglas_peucker(h, tol_deg))
        .filter(|h| h.len() >= 3)
        .collect();
    // 简化可能拉出跨日界弦，再切一次
    ringsets_from_polygon(exterior, holes)
}

fn douglas_peucker(pts: &[[f32; 2]], tol_deg: f64) -> Vec<[f32; 2]> {
    if pts.len() < 3 || tol_deg <= 0.0 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    dp_recurse(pts, 0, pts.len() - 1, tol_deg * tol_deg, &mut keep);
    pts.iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, p)| *p)
        .collect()
}

fn dp_recurse(pts: &[[f32; 2]], i0: usize, i1: usize, tol2: f64, keep: &mut [bool]) {
    if i1 <= i0 + 1 {
        return;
    }
    let a = pts[i0];
    let b = pts[i1];
    let mut max_d2 = 0.0_f64;
    let mut max_i = i0;
    for i in (i0 + 1)..i1 {
        let d2 = dist2_point_seg(pts[i], a, b);
        if d2 > max_d2 {
            max_d2 = d2;
            max_i = i;
        }
    }
    if max_d2 > tol2 {
        keep[max_i] = true;
        dp_recurse(pts, i0, max_i, tol2, keep);
        dp_recurse(pts, max_i, i1, tol2, keep);
    }
}

fn dist2_point_seg(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f64 {
    let px = p[0] as f64;
    let py = p[1] as f64;
    let ax = a[0] as f64;
    let ay = a[1] as f64;
    let bx = b[0] as f64;
    let by = b[1] as f64;
    let dx = bx - ax;
    let dy = by - ay;
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-18 {
        0.0
    } else {
        ((px - ax) * dx + (py - ay) * dy) / len2
    }
    .clamp(0.0, 1.0);
    let qx = ax + t * dx;
    let qy = ay + t * dy;
    let ex = px - qx;
    let ey = py - qy;
    ex * ex + ey * ey
}

fn viewport_bounds(center_lon: f64, center_lat: f64, zoom: f64, rect: Rect) -> (f64, f64, f64, f64) {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.left_bottom(),
        rect.right_bottom(),
    ];
    let mut min_lon = f64::MAX;
    let mut min_lat = f64::MAX;
    let mut max_lon = f64::MIN;
    let mut max_lat = f64::MIN;
    let (cx, cy) = projection::lonlat_to_world(center_lon, center_lat, zoom);
    for c in corners {
        let wx = cx + (c.x - rect.center().x) as f64;
        let wy = cy + (c.y - rect.center().y) as f64;
        let (lon, lat) = projection::world_to_lonlat(wx, wy, zoom);
        min_lon = min_lon.min(lon);
        min_lat = min_lat.min(lat);
        max_lon = max_lon.max(lon);
        max_lat = max_lat.max(lat);
    }
    let pad_lon = (max_lon - min_lon) * 0.05;
    let pad_lat = (max_lat - min_lat) * 0.05;
    (
        min_lon - pad_lon,
        min_lat - pad_lat,
        max_lon + pad_lon,
        max_lat + pad_lat,
    )
}

/// 相对视口中心连续解缠后投影；经度或屏幕跳变时拆段，禁止跨日界连线。
/// `lon_shift`：世界副本偏移（±360°…），用于低缩放横向铺满；球体忽略。
fn project_segments(
    ring: &[[f32; 2]],
    view: &ViewCtx,
    min_seg_px: f32,
    max_jump_px: f32,
    lon_shift: f64,
) -> Vec<Vec<Pos2>> {
    let min2 = min_seg_px * min_seg_px;
    let jump2 = max_jump_px * max_jump_px;
    let mut segments: Vec<Vec<Pos2>> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::with_capacity(ring.len().min(256));

    if view.mode.is_globe() {
        for p in ring {
            let lon = p[0] as f64;
            let lat = p[1] as f64;
            match view.project(lon, lat) {
                Some(sp) => {
                    if let Some(last) = cur.last() {
                        let dx = sp.x - last.x;
                        let dy = sp.y - last.y;
                        let d2 = dx * dx + dy * dy;
                        if d2 < min2 {
                            continue;
                        }
                        if d2 > jump2 {
                            if cur.len() >= 2 {
                                segments.push(std::mem::take(&mut cur));
                            } else {
                                cur.clear();
                            }
                        }
                    }
                    cur.push(sp);
                }
                None => {
                    if cur.len() >= 2 {
                        segments.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                }
            }
        }
        if cur.len() >= 2 {
            segments.push(cur);
        }
        return segments;
    }

    let center_lon = view.center_lon;
    let center_lat = view.center_lat;
    let zoom = view.zoom;
    let rect = view.rect;
    let (cx, cy) = projection::lonlat_to_world(center_lon, center_lat, zoom);
    let mut prev_lon: Option<f64> = None;

    for p in ring {
        let lat = p[1] as f64;
        let mut lon = projection::wrap_lon_near(p[0] as f64, center_lon) + lon_shift;
        if let Some(prev) = prev_lon {
            while lon - prev > 180.0 {
                lon -= 360.0;
            }
            while prev - lon > 180.0 {
                lon += 360.0;
            }
            if (lon - prev).abs() > 180.0 {
                if cur.len() >= 2 {
                    segments.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                lon = projection::wrap_lon_near(p[0] as f64, center_lon) + lon_shift;
            }
        }

        let (wx, wy) = projection::lonlat_to_world(lon, lat, zoom);
        let sp = rect.center()
            + egui::Vec2::new((wx - cx) as f32, (wy - cy) as f32);

        if let Some(last) = cur.last() {
            let dx = sp.x - last.x;
            let dy = sp.y - last.y;
            let d2 = dx * dx + dy * dy;
            if d2 < min2 {
                prev_lon = Some(lon);
                continue;
            }
            if d2 > jump2 {
                if cur.len() >= 2 {
                    segments.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
        cur.push(sp);
        prev_lon = Some(lon);
    }
    if cur.len() >= 2 {
        segments.push(cur);
    }
    segments
}

/// 经度归一到 (-180, 180]。
fn norm_lon_f32(lon: f32) -> f32 {
    let mut x = (lon as f64 + 180.0).rem_euclid(360.0) - 180.0;
    if x <= -180.0 {
        x = 180.0;
    }
    x as f32
}

/// 是否存在跨日界边。用原始经度差；±180 之间的缝边不算跨越。
fn has_antimeridian_edge(pts: &[[f32; 2]]) -> bool {
    if pts.len() < 2 {
        return false;
    }
    let n = pts.len();
    for i in 0..n {
        let a = pts[i][0];
        let b = pts[(i + 1) % n][0];
        if a.abs() >= 179.999 && b.abs() >= 179.999 {
            continue;
        }
        if (a as f64 - b as f64).abs() > 180.0 {
            return true;
        }
    }
    false
}

/// 将环上经度解缠为连续轨迹（相邻点 |Δlon| ≤ 180°）。
fn unwrap_ring(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if pts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(pts.len());
    let mut prev = norm_lon_f32(pts[0][0]);
    out.push([prev, pts[0][1]]);
    for p in pts.iter().skip(1) {
        let mut lon = norm_lon_f32(p[0]);
        while (lon as f64) - (prev as f64) > 180.0 {
            lon -= 360.0;
        }
        while (prev as f64) - (lon as f64) > 180.0 {
            lon += 360.0;
        }
        out.push([lon, p[1]]);
        prev = lon;
    }
    out
}

fn unwrapped_lon_span(pts: &[[f32; 2]]) -> f32 {
    if pts.is_empty() {
        return 0.0;
    }
    let u = unwrap_ring(pts);
    let mut min_lon = f32::MAX;
    let mut max_lon = f32::MIN;
    for p in &u {
        min_lon = min_lon.min(p[0]);
        max_lon = max_lon.max(p[0]);
    }
    max_lon - min_lon
}

/// 在跨日界边上插入 (180,lat)/(-180,lat) 对并拆环；绕地过半的环直接丢弃。
fn split_closed_ring(pts: &[[f32; 2]]) -> Vec<Vec<[f32; 2]>> {
    if pts.len() < 3 {
        return Vec::new();
    }

    let crosses = has_antimeridian_edge(pts);
    let out_norm: Vec<[f32; 2]> = pts.iter().map(|p| [norm_lon_f32(p[0]), p[1]]).collect();

    // 无跨日界边，或绕地大环（南极式）：整环保留，paint 侧拆段描边
    if !crosses || unwrapped_lon_span(pts) > 180.0 + 1e-3 {
        return vec![out_norm];
    }


    let n = pts.len();
    let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();

    for i in 0..n {
        let a = [norm_lon_f32(pts[i][0]), pts[i][1]];
        let b = [norm_lon_f32(pts[(i + 1) % n][0]), pts[(i + 1) % n][1]];
        if cur.is_empty() {
            cur.push(a);
        }
        if (a[0] as f64 - b[0] as f64).abs() > 180.0 {
            let (cut_a, cut_b) = dateline_pair(a, b);
            cur.push(cut_a);
            if cur.len() >= 3 {
                pieces.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur.push(cut_b);
            if i + 1 < n {
                cur.push(b);
            }
        } else if i + 1 < n {
            cur.push(b);
        }
    }
    if cur.len() >= 3 {
        pieces.push(cur);
    }

    // 环起点落在某一侧中部时，首尾两段同属一侧，可安全拼回
    if pieces.len() >= 2 {
        let mut merged = pieces[pieces.len() - 1].clone();
        let head = pieces[0].clone();
        merged.extend(head.iter().skip(1).copied());
        if merged.len() >= 3
            && unwrapped_lon_span(&merged) <= 180.0 + 1e-3
            && !has_antimeridian_edge(&merged)
        {
            pieces.pop();
            pieces.remove(0);
            pieces.insert(0, merged);
        }
    }

    pieces
        .into_iter()
        .filter(|p| p.len() >= 3)
        .filter(|p| unwrapped_lon_span(p) <= 180.0 + 1e-3)
        .filter(|p| !has_antimeridian_edge(p))
        .collect()
}

/// 跨日界边的两端缝点：西侧片用 +180，东侧片用 -180，保证片内 |Δlon|≤180。
fn dateline_pair(a: [f32; 2], b: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    // 在解缠空间插值纬度
    let mut a_lon = a[0];
    let mut b_lon = b[0];
    if a_lon - b_lon > 180.0 {
        b_lon += 360.0;
    } else if b_lon - a_lon > 180.0 {
        a_lon += 360.0;
    }
    // 目标缝：落在 a_lon→b_lon 之间的 ±180+360k
    let seam = if a_lon < b_lon {
        (a_lon / 360.0).ceil() * 360.0 - 180.0
    } else {
        (b_lon / 360.0).ceil() * 360.0 - 180.0
    };
    let t = if (b_lon - a_lon).abs() < 1e-9 {
        0.5
    } else {
        ((seam - a_lon) / (b_lon - a_lon)).clamp(0.0, 1.0)
    };
    let lat = a[1] + t * (b[1] - a[1]);
    if a[0] > 0.0 {
        ([180.0, lat], [-180.0, lat])
    } else {
        ([-180.0, lat], [180.0, lat])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basemap_mode_parse() {
        assert_eq!(BasemapMode::parse("vector"), BasemapMode::Vector);
        assert_eq!(BasemapMode::parse("TILES"), BasemapMode::Tiles);
    }

    #[test]
    fn split_fiji_like_ring_across_dateline() {
        // 跨日界线的小环（类似斐济）：应切成两片，且均可填色
        let ring = [
            [179.0, -16.0],
            [179.5, -16.0],
            [179.5, -17.0],
            [-179.5, -17.0],
            [-179.5, -16.0],
            [-179.0, -16.0],
            [179.0, -16.0],
        ];
        let mut pts = ring.to_vec();
        pts.pop(); // 去闭合点
        assert!(has_antimeridian_edge(&pts), "fixture must cross dateline");
        let parts = split_closed_ring(&pts);
        assert!(
            parts.len() >= 2,
            "expected split, got {} parts: {:?}",
            parts.len(),
            parts
        );
        for p in &parts {
            assert!(p.len() >= 3);
            assert!(unwrapped_lon_span(p) <= 180.0 + 1e-2);
            assert!(!has_antimeridian_edge(p));
        }
        let sets = ringsets_from_polygon(pts, Vec::new());
        assert!(!sets.is_empty());
        assert!(sets.iter().all(|s| s.may_fill()));
    }

    #[test]
    fn keep_full_world_spanning_ring_for_stroke() {
        // 跨度 >180° 但无跨日界边（南极式）→ 保留供描边
        let mut pts = Vec::new();
        for i in 0..36 {
            let lon = -180.0 + i as f32 * 10.0;
            pts.push([lon, -80.0]);
        }
        let parts = split_closed_ring(&pts);
        assert_eq!(parts.len(), 1);
        assert!(RingSet::from_parts(parts[0].clone(), Vec::new()).is_some());
    }

    #[test]
    fn plain_ring_unchanged() {
        let pts = vec![
            [100.0, 20.0],
            [110.0, 20.0],
            [110.0, 30.0],
            [100.0, 30.0],
        ];
        let parts = split_closed_ring(&pts);
        assert_eq!(parts.len(), 1);
        let set = RingSet::from_parts(parts[0].clone(), Vec::new()).unwrap();
        assert!(set.may_fill());
        assert_eq!(set.tris.len(), 6); // 四边形 → 2 个三角形
    }

    #[test]
    fn triangulate_ring_with_hole() {
        let exterior = vec![
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ];
        let hole = vec![
            [2.0, 2.0],
            [2.0, 8.0],
            [8.0, 8.0],
            [8.0, 2.0],
        ];
        let tris = triangulate_ring(&exterior, &[hole]);
        assert!(tris.len() >= 12); // 外环+孔至少若干三角形
        assert!(tris.len() % 3 == 0);
        assert!(tris.iter().all(|&i| i < 8));
    }
}
