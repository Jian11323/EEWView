//! 行政区矢量底图（暗色要石 / 亮色浅填）。
//!
//! 优先 `geodata/kanameishi/`（要石 TopoJSON / 展开 GeoJSON）；否则回退 Globalmap + 省市区划。
//! 加载在后台线程分两阶段（世界 → 区划），主线程 `poll` 合并。

use crate::projection;
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
            // KA / RhythmQuake：land #393939，border #BBBBBB
            land_fill: Color32::from_rgb(0x39, 0x39, 0x39),
            world_border: Color32::from_rgb(0xBB, 0xBB, 0xBB),
            admin_border: Color32::from_rgb(0xBB, 0xBB, 0xBB),
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

    /// `auto`：暗色→要石矢量，亮色→地形瓦片；其它值按 `parse`。
    pub fn resolve(configured: &str, theme: &str) -> Self {
        match configured.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => {
                if theme.eq_ignore_ascii_case("light") {
                    Self::Tiles
                } else {
                    Self::Vector
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
}

impl RingSet {
    fn from_parts(exterior: Vec<[f32; 2]>, holes: Vec<Vec<[f32; 2]>>) -> Option<Self> {
        if exterior.len() < 3 {
            return None;
        }
        let lon_span = unwrapped_lon_span(&exterior);
        let (min_lon, min_lat, max_lon, max_lat) = bbox_of(&exterior);
        Some(Self {
            exterior,
            holes,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            lon_span,
        })
    }

    /// egui PathShape 填色对凹多边形会扇出射线；仅极简单小环才填。
    fn may_fill(&self) -> bool {
        self.lon_span <= 40.0
            && self.exterior.len() <= 24
            && !has_antimeridian_edge(&self.exterior)
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
        self.world = LayerLods {
            coarse: Vec::new(),
            medium: Vec::new(),
            fine: Vec::new(),
        };
        self.regional = LayerLods {
            coarse: Vec::new(),
            medium: Vec::new(),
            fine: Vec::new(),
        };
        self.national = LayerLods {
            coarse: Vec::new(),
            medium: Vec::new(),
            fine: Vec::new(),
        };
        self.world_ready = false;
        self.regional_ready = false;
        self.state = BasemapLoadState::LoadingWorld;
        self.status = "正在加载世界底图…".into();

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

                let (regional_fine, r_files, r_skip) = load_regional_rings(&dir);
                let regional_n = regional_fine.len();
                let national_fine = dissolve_national_outlines(&regional_fine);
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
        &self,
        ui: &mut Ui,
        rect: Rect,
        center_lon: f64,
        center_lat: f64,
        zoom: f64,
        fill_land: bool,
        style: VectorStyle,
    ) {
        if !self.world_ready && !self.regional_ready {
            return;
        }

        let world_px = projection::world_size(zoom) as f32;
        // 视口宽超过一整圈世界时，经纬度包围盒会因 wrap 塌缩；改为全球可见
        let full_wrap = rect.width() >= world_px * 0.92;
        let (min_lon, min_lat, max_lon, max_lat) = if full_wrap {
            (-180.0, -85.0, 180.0, 85.0)
        } else {
            viewport_bounds(center_lon, center_lat, zoom, rect)
        };
        let painter = ui.painter_at(rect);
        let mut shapes = Vec::with_capacity(512);
        // 超过约 80° 经度的边视为跳变（宁拆勿穿）
        let max_jump_px = world_px * (80.0 / 360.0);
        // 低缩放 / 宽窗口：横向循环多画几份世界（经度 ±360°），避免两侧空白
        let half_copies = ((rect.width() * 0.5 / world_px.max(1.0)).ceil() as i32 + 1)
            .clamp(1, 6);

        for k in -half_copies..=half_copies {
            let lon_shift = f64::from(k) * 360.0;

            // 1) 世界轮廓：连续解缠描边。egui 凹多边形填色会出射线，默认不填。
            if !self.world.fine.is_empty() {
                let rings = self.world.pick(zoom, 4.0, 6.0);
                let min_seg = if zoom < 4.0 { 2.2_f32 } else { 1.4_f32 };
                let stroke = Stroke::new(0.9_f32, style.world_border);
                paint_rings(
                    &mut shapes,
                    rings,
                    center_lon,
                    center_lat,
                    zoom,
                    rect,
                    min_lon + lon_shift,
                    min_lat,
                    max_lon + lon_shift,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    stroke,
                    false, // 世界层不填色
                    false,
                    style.land_fill,
                    lon_shift,
                );
            }

            // 2) 中日：远距只画国界轮廓；放大后才叠省界/都道府县
            if zoom < ZOOM_REGIONAL {
                if !self.national.fine.is_empty() {
                    let rings = self.national.pick(zoom, 3.0, 4.0);
                    let stroke = Stroke::new(0.95_f32, style.world_border);
                    paint_rings(
                        &mut shapes,
                        rings,
                        center_lon,
                        center_lat,
                        zoom,
                        rect,
                        min_lon + lon_shift,
                        min_lat,
                        max_lon + lon_shift,
                        max_lat,
                        2.2,
                        max_jump_px,
                        stroke,
                        false,
                        false,
                        style.land_fill,
                        lon_shift,
                    );
                }
            } else if !self.regional.fine.is_empty() {
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
                    center_lon,
                    center_lat,
                    zoom,
                    rect,
                    min_lon + lon_shift,
                    min_lat,
                    max_lon + lon_shift,
                    max_lat,
                    min_seg,
                    max_jump_px,
                    stroke,
                    false,
                    zoom >= ZOOM_REGIONAL_FINE,
                    style.land_fill,
                    lon_shift,
                );
            }
        }

        let _ = fill_land;
        painter.extend(shapes);
    }
}

fn paint_rings(
    shapes: &mut Vec<egui::Shape>,
    rings: &[RingSet],
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    rect: Rect,
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
            center_lon,
            center_lat,
            zoom,
            rect,
            min_seg_px,
            max_jump_px,
            lon_shift,
        );
        let nseg = segments.len();
        let do_fill = fill
            && ring.may_fill()
            && nseg == 1
            && segments.first().is_some_and(|s| s.len() >= 3);
        for mut pts in segments {
            if pts.len() < 2 {
                continue;
            }
            // 绝不使用 closed=true（首尾跨屏时会拉出射线）；需要闭合时只短距补点
            if do_fill && pts.len() >= 3 {
                let a = pts[0];
                let b = pts[pts.len() - 1];
                let dx = a.x - b.x;
                let dy = a.y - b.y;
                if dx * dx + dy * dy < max_jump_px * max_jump_px * 0.25 {
                    pts.push(a);
                }
            }
            shapes.push(
                egui::epaint::PathShape {
                    points: pts,
                    closed: false,
                    fill: if do_fill {
                        land_fill
                    } else {
                        Color32::TRANSPARENT
                    },
                    stroke: egui::epaint::PathStroke::new(stroke.width, stroke.color),
                }
                .into(),
            );
        }

        if draw_holes {
            for hole in &ring.holes {
                let hsegs = project_segments(
                    hole,
                    center_lon,
                    center_lat,
                    zoom,
                    rect,
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

/// 全球层：优先要石 `kanameishi/`，否则 Globalmap + `world/` 补丁。
fn load_world_rings(dir: &Path) -> (Vec<RingSet>, usize, usize) {
    let mut world_fine = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;

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

    let mut saw_patch = false;
    for entry in world_entries {
        if globalmap.as_ref().is_some_and(|g| g == &entry) {
            continue;
        }
        let patch = is_cn_boundary_patch(&entry);
        if use_globalmap && !patch {
            continue;
        }
        if patch {
            saw_patch = true;
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

    if !saw_patch {
        world_fine.extend(builtin_cn_boundary_patches());
        info!("using builtin cn boundary patches (藏南 / 钓鱼岛)");
    }

    (world_fine, files, skipped)
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

/// 内置藏南 / 钓鱼岛补丁（无补丁文件时使用）。
fn builtin_cn_boundary_patches() -> Vec<RingSet> {
    let zangnan: &[[f32; 2]] = &[
        [91.0, 28.15],
        [92.2, 28.35],
        [93.5, 28.45],
        [95.0, 28.55],
        [96.5, 28.35],
        [97.35, 28.05],
        [97.35, 27.2],
        [96.2, 26.85],
        [94.8, 26.7],
        [93.2, 26.85],
        [91.8, 27.05],
        [91.0, 27.55],
        [91.0, 28.15],
    ];
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
    if let Some(r) = ringset_from_lonlat(zangnan) {
        out.push(r);
    }
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
/// `lon_shift`：世界副本偏移（±360°…），用于低缩放横向铺满。
fn project_segments(
    ring: &[[f32; 2]],
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    rect: Rect,
    min_seg_px: f32,
    max_jump_px: f32,
    lon_shift: f64,
) -> Vec<Vec<Pos2>> {
    let min2 = min_seg_px * min_seg_px;
    let jump2 = max_jump_px * max_jump_px;
    let (cx, cy) = projection::lonlat_to_world(center_lon, center_lat, zoom);
    let mut segments: Vec<Vec<Pos2>> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::with_capacity(ring.len().min(256));
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
            // 连续解缠后仍超过 180°：地理上不该相连 → 拆段
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
        assert!(RingSet::from_parts(parts[0].clone(), Vec::new())
            .unwrap()
            .may_fill());
    }
}
