//! 行政区矢量底图（暗色矢量 / 亮色浅填）。
//!
//! 优先 `geodata/globalquake/`（GlobalQuake Special 藏南修复国界 + 中日一级区划）；
//! 否则要石 `kanameishi/`；再回退 Globalmap + 省市区划。
//! 加载在后台线程分两阶段（世界 → 区划），主线程 `poll` 合并。

use crate::globe::zoom_to_scroll;
use crate::projection::{self, ViewCtx};
use crate::topojson;
use egui::{Color32, Pos2, Rect, Stroke, Ui};
use jian_core::palette::color_for;
use jian_core::EewReport;
use jian_util::haversine_km;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use tracing::{info, warn};

/// 海洋底色（暗色轮廓图：近黑）
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
            // 轮廓图：陆地不填色；描边浅灰
            land_fill: Color32::from_rgb(0x4A, 0x4A, 0x4E),
            world_border: Color32::from_rgb(0xC8, 0xC8, 0xCC),
            admin_border: Color32::from_rgb(0xA0, 0xA0, 0xA4),
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
/// 区县描边起始缩放；过大时（且 LOD 未简化）会拖死主线程。
const ZOOM_REGIONAL: f64 = 6.5;
const ZOOM_REGIONAL_MED: f64 = 8.0;
const ZOOM_REGIONAL_FINE: f64 = 9.0;

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

    /// `auto`：亮色→地形瓦片；暗色→矢量（GQ 风格：海洋底 + 陆地 Path 填色）。
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

    /// 环数超过 `share_above` 时跳过简化，`pick` 回退到 fine（避免上千区县双重简化过久）。
    fn from_fine_or_shared(
        fine: Vec<RingSet>,
        coarse_tol: f64,
        medium_tol: f64,
        share_above: usize,
    ) -> Self {
        if fine.len() > share_above {
            Self {
                coarse: Vec::new(),
                medium: Vec::new(),
                fine,
            }
        } else {
            Self::from_fine(fine, coarse_tol, medium_tol)
        }
    }

    fn pick(&self, zoom: f64, z_med: f64, z_fine: f64) -> &[RingSet] {
        // 区县量大时 coarse/medium 可能为空（跳过简化）；禁止回退到 fine，
        // 否则远距也会每帧描几千区划，亮色矢量会卡死/崩。
        if zoom < z_med {
            if !self.coarse.is_empty() {
                self.coarse.as_slice()
            } else {
                &[]
            }
        } else if zoom < z_fine {
            if !self.medium.is_empty() {
                self.medium.as_slice()
            } else if !self.coarse.is_empty() {
                self.coarse.as_slice()
            } else {
                &[]
            }
        } else if !self.fine.is_empty() {
            self.fine.as_slice()
        } else if !self.medium.is_empty() {
            self.medium.as_slice()
        } else {
            self.coarse.as_slice()
        }
    }
}

enum LoadPacket {
    World {
        lods: LayerLods,
        rings: usize,
        files: usize,
        skipped: usize,
        /// GlobalQuake Special 分层（有则球体按 scroll 绘制）
        gq: Option<GqLayers>,
    },
    Regional {
        lods: LayerLods,
        national: LayerLods,
        rings: usize,
        files: usize,
        skipped: usize,
    },
}

/// 对齐 GQ `GlobePanel` / `Regions` 的图层集合。
#[derive(Clone, Default)]
struct GqLayers {
    md: Vec<RingSet>,
    hd_filtered: Vec<RingSet>,
    uhd_filtered: Vec<RingSet>,
    cn: Vec<RingSet>,
    jp: Vec<RingSet>,
    us: Vec<RingSet>,
    ak: Vec<RingSet>,
    nz: Vec<RingSet>,
    hw: Vec<RingSet>,
    it: Vec<RingSet>,
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

/// 烈度区划填色缓存（事件+视口不变时不每帧重算）。
struct IntensityPaintCache {
    key: u64,
    shapes: Vec<egui::Shape>,
}

pub struct VectorBasemap {
    world: LayerLods,
    /// 中日省界 / 都道府县（近距）
    regional: LayerLods,
    /// 中日外轮廓（远距；由区划边界溶解得到）
    national: LayerLods,
    /// GlobalQuake Special 完整图层（优先）
    gq: Option<GqLayers>,
    world_ready: bool,
    regional_ready: bool,
    state: BasemapLoadState,
    status: String,
    gen: u64,
    rx: Option<Receiver<(u64, LoadPacket)>>,
    loaded_dir: Option<PathBuf>,
    paint_cache: Option<VectorPaintCache>,
    intensity_cache: Option<IntensityPaintCache>,
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
            gq: None,
            world_ready: false,
            regional_ready: false,
            state: BasemapLoadState::Empty,
            status: String::new(),
            gen: 0,
            rx: None,
            loaded_dir: None,
            paint_cache: None,
            intensity_cache: None,
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
                let (world_lods, world_n, w_files, w_skip, gq) = load_world_lods(&dir);
                if tx
                    .send((
                        gen,
                        LoadPacket::World {
                            lods: world_lods,
                            rings: world_n,
                            files: w_files,
                            skipped: w_skip,
                            gq,
                        },
                    ))
                    .is_err()
                {
                    return;
                }

                // 平面区划：中国地市/区县扫盘 + 日本都道府县（精细烈度填色）
                let (mut regional_fine, r_files, r_skip) = load_regional_rings(&dir);
                // 藏南：GQ Special 国界已含；区划叠在世界层之上。不另画凸包。
                let diaoyu = builtin_diaoyu_patches();
                // 远距外轮廓仍用 GQ/要石一级区划溶解（比三千区县溶解更快更稳）
                let mut dissolve_src = load_coarse_admin_for_dissolve(&dir);
                if dissolve_src.is_empty() {
                    dissolve_src = regional_fine.clone();
                }
                dissolve_src.extend(diaoyu.iter().cloned());
                let national_fine = dissolve_national_outlines(&dissolve_src);
                regional_fine.extend(diaoyu);
                let regional_n = regional_fine.len();
                let national_lods = LayerLods::from_fine(national_fine, 0.25, 0.08);
                // 区县数量大：跳过 Visvalingam 全量简化，三档共用 fine（后台加载更快）
                let regional_lods = LayerLods::from_fine_or_shared(regional_fine, 0.08, 0.02, 800);
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
            self.intensity_cache = None;
            match packet {
                LoadPacket::World {
                    lods,
                    rings,
                    files,
                    skipped,
                    gq,
                } => {
                    self.world = lods;
                    self.gq = gq;
                    self.world_ready = !self.world.fine.is_empty()
                        || self.gq.as_ref().is_some_and(|g| !g.md.is_empty());
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
        skip_world_stroke: bool,
    ) {
        if !self.world_ready && !self.regional_ready {
            return;
        }

        let rect = view.rect;
        let zoom = view.zoom;
        let painter = ui.painter_at(rect);
        let cache_key = paint_cache_key(view, fill_land, style, self.gen, skip_world_stroke);
        if let Some(c) = self.paint_cache.as_ref() {
            if c.key == cache_key {
                painter.extend(c.shapes.clone());
                return;
            }
        }
        let mut shapes = Vec::with_capacity(512);

        if view.mode.is_globe() {
            // 对齐 GQ GlobePanel：按 scroll 切换 MD / HD·UHD(过滤) + 区划填色描边
            paint_globe_gq_style(&mut shapes, self, view, fill_land, style);
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

    /// 陆地贴边的衰减烈度：区划耳切三角按震中距估档，大边再四分（有预算+缓存）。
    ///
    /// 外缘跟区划走（锯齿陆地，对齐 Hikari 类参考）；区内随距离分档。
    /// 不做环半径多边形裁切（易卡死）。无命中返回 `false` 由上层画同心环。
    pub fn paint_intensity_regions(
        &mut self,
        ui: &mut Ui,
        view: &ViewCtx,
        ev: &EewReport,
        selected: bool,
    ) -> bool {
        if ev.is_cancel || ev.event_id.starts_with("station:") {
            return false;
        }
        if !ev.magnitude.is_finite() || ev.magnitude <= 0.0 {
            return false;
        }
        let layers = self.intensity_region_layers();
        if layers.is_empty() {
            return false;
        }

        let cache_key = intensity_cache_key(view, ev, selected, self.gen);
        if let Some(c) = self.intensity_cache.as_ref() {
            if c.key == cache_key {
                if c.shapes.is_empty() {
                    return false;
                }
                ui.painter_at(view.rect).extend(c.shapes.clone());
                return true;
            }
        }

        let rings = jian_core::intensity_rings(ev.intensity_kind, ev.magnitude, ev.depth_km);
        let Some(&(_, max_r_km)) = rings.last() else {
            self.intensity_cache = Some(IntensityPaintCache {
                key: cache_key,
                shapes: Vec::new(),
            });
            return false;
        };
        let cull_km = max_r_km.max(25.0) + 80.0;
        let rect = view.rect;
        let fill_alpha = if selected { 120_u8 } else { 88_u8 };
        let globe = view.mode.is_globe();
        let world_px = if globe {
            rect.width().max(rect.height())
        } else {
            projection::world_size(view.zoom) as f32
        };
        let max_edge_px = rect.width().hypot(rect.height()).max(64.0);
        let (min_lon, min_lat, max_lon, max_lat) = if globe {
            (-180.0, -85.0, 180.0, 85.0)
        } else {
            let full_wrap = rect.width() >= world_px * 0.92;
            if full_wrap {
                (-180.0, -85.0, 180.0, 85.0)
            } else {
                viewport_bounds(view.center_lon, view.center_lat, view.zoom, rect)
            }
        };
        let lon_shifts: Vec<f64> = if globe {
            vec![0.0]
        } else if min_lon < -150.0 || max_lon > 150.0 || view.center_lon.abs() > 150.0 {
            vec![-360.0, 0.0, 360.0]
        } else {
            vec![0.0]
        };

        let mut meshes: HashMap<u8, egui::epaint::Mesh> = HashMap::new();
        let mut tri_budget = INTENSITY_TRI_BUDGET;

        for layer in layers {
            for ring in layer {
                if tri_budget == 0 {
                    break;
                }
                if !ring.may_fill() || ring.tris.is_empty() {
                    continue;
                }
                let diag = bbox_half_diag_km(ring);
                let (clon, clat) = ring_centroid(ring);
                let cen_dist = haversine_km(ev.longitude, ev.latitude, clon, clat);
                if cen_dist > cull_km + diag {
                    continue;
                }
                let verts = ring_lonlat_vertices(ring);
                if verts.len() < 3 {
                    continue;
                }
                for &lon_shift in &lon_shifts {
                    if !ring_visible(
                        ring,
                        min_lon - lon_shift,
                        min_lat,
                        max_lon - lon_shift,
                        max_lat,
                    ) {
                        continue;
                    }
                    // 包围盒档位一致：整环同色（省掉逐三角）
                    let d_lo = if point_in_bbox(ev.longitude, ev.latitude, ring)
                        && point_in_ring(ev.longitude, ev.latitude, &ring.exterior)
                    {
                        0.0
                    } else {
                        (cen_dist - diag).max(0.0)
                    };
                    let d_hi = cen_dist + diag;
                    let lv_lo = jian_core::estimate_at_site(
                        ev.intensity_kind,
                        ev.magnitude,
                        ev.depth_km,
                        d_lo,
                    )
                    .0;
                    let lv_hi = jian_core::estimate_at_site(
                        ev.intensity_kind,
                        ev.magnitude,
                        ev.depth_km,
                        d_hi,
                    )
                    .0;
                    if lv_lo == lv_hi {
                        if lv_lo < 1 {
                            continue;
                        }
                        let rgb = color_for(ev.intensity_kind, lv_lo);
                        let fill =
                            Color32::from_rgba_unmultiplied(rgb.r, rgb.g, rgb.b, fill_alpha);
                        if let Some(mesh) =
                            mesh_fill_intensity_ring(ring, view, lon_shift, fill, max_edge_px)
                        {
                            let n = mesh.indices.len() / 3;
                            append_mesh(meshes.entry(lv_lo).or_default(), mesh);
                            tri_budget = tri_budget.saturating_sub(n);
                        }
                        continue;
                    }
                    for tri in ring.tris.chunks_exact(3) {
                        if tri_budget == 0 {
                            break;
                        }
                        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
                        if i0 >= verts.len() || i1 >= verts.len() || i2 >= verts.len() {
                            continue;
                        }
                        paint_intensity_tri_land(
                            &mut meshes,
                            &mut tri_budget,
                            view,
                            lon_shift,
                            max_edge_px,
                            fill_alpha,
                            ev,
                            verts[i0],
                            verts[i1],
                            verts[i2],
                            0,
                        );
                    }
                }
            }
        }

        let mut shapes: Vec<egui::Shape> = Vec::with_capacity(meshes.len());
        for (_lv, mesh) in meshes {
            if !mesh.indices.is_empty() {
                shapes.push(egui::Shape::mesh(mesh));
            }
        }
        let ok = !shapes.is_empty();
        if ok {
            ui.painter_at(rect).extend(shapes.clone());
        }
        self.intensity_cache = Some(IntensityPaintCache {
            key: cache_key,
            shapes,
        });
        ok
    }

    fn intensity_region_layers(&self) -> Vec<&[RingSet]> {
        let mut out: Vec<&[RingSet]> = Vec::new();
        if !self.regional.fine.is_empty() {
            out.push(self.regional.fine.as_slice());
        }
        if let Some(gq) = &self.gq {
            for slice in [
                gq.us.as_slice(),
                gq.ak.as_slice(),
                gq.nz.as_slice(),
                gq.hw.as_slice(),
                gq.it.as_slice(),
            ] {
                if !slice.is_empty() {
                    out.push(slice);
                }
            }
            if self.regional.fine.is_empty() {
                for slice in [gq.cn.as_slice(), gq.jp.as_slice()] {
                    if !slice.is_empty() {
                        out.push(slice);
                    }
                }
            }
        }
        out
    }
}

fn intensity_cache_key(view: &ViewCtx, ev: &EewReport, selected: bool, gen: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    gen.hash(&mut h);
    ev.event_id.hash(&mut h);
    ev.magnitude.to_bits().hash(&mut h);
    ev.depth_km.to_bits().hash(&mut h);
    match ev.intensity_kind {
        jian_core::IntensityKind::JmaShindo => 0_u8,
        jian_core::IntensityKind::CnIntensity => 1_u8,
    }
    .hash(&mut h);
    selected.hash(&mut h);
    // 视口量化：轻微平移不重算
    ((view.center_lon * 20.0).round() as i32).hash(&mut h);
    ((view.center_lat * 20.0).round() as i32).hash(&mut h);
    ((view.zoom * 4.0).round() as i32).hash(&mut h);
    (view.rect.width() as i32 / 32).hash(&mut h);
    (view.rect.height() as i32 / 32).hash(&mut h);
    view.mode.is_globe().hash(&mut h);
    h.finish()
}

fn ring_centroid(ring: &RingSet) -> (f64, f64) {
    let n = ring.exterior.len().max(1) as f64;
    let mut slon = 0.0_f64;
    let mut slat = 0.0_f64;
    for p in &ring.exterior {
        slon += p[0] as f64;
        slat += p[1] as f64;
    }
    (slon / n, slat / n)
}

fn ring_lonlat_vertices(ring: &RingSet) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(
        ring.exterior.len() + ring.holes.iter().map(|h| h.len()).sum::<usize>(),
    );
    out.extend_from_slice(&ring.exterior);
    for hole in &ring.holes {
        if hole.len() >= 3 {
            out.extend_from_slice(hole);
        }
    }
    out
}

/// 单帧陆地烈度三角上限（缓存命中后不再算）。
const INTENSITY_TRI_BUDGET: usize = 18_000;
const INTENSITY_TRI_SPLIT_KM: f64 = 42.0;
const INTENSITY_TRI_MAX_DEPTH: u8 = 4;

/// 陆地三角：边过长且跨档时四分；否则按质心距填一色。
fn paint_intensity_tri_land(
    meshes: &mut HashMap<u8, egui::epaint::Mesh>,
    budget: &mut usize,
    view: &ViewCtx,
    lon_shift: f64,
    max_edge_px: f32,
    fill_alpha: u8,
    ev: &EewReport,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    depth: u8,
) {
    if *budget == 0 {
        return;
    }
    let dist = |p: [f32; 2]| haversine_km(ev.longitude, ev.latitude, p[0] as f64, p[1] as f64);
    let e_ab = haversine_km(a[0] as f64, a[1] as f64, b[0] as f64, b[1] as f64);
    let e_bc = haversine_km(b[0] as f64, b[1] as f64, c[0] as f64, c[1] as f64);
    let e_ca = haversine_km(c[0] as f64, c[1] as f64, a[0] as f64, a[1] as f64);
    let max_e = e_ab.max(e_bc).max(e_ca);

    let da = dist(a);
    let db = dist(b);
    let dc = dist(c);
    let mut d_lo = da.min(db).min(dc);
    let d_hi = da.max(db).max(dc);
    if point_in_ring(ev.longitude, ev.latitude, &[a, b, c]) {
        d_lo = 0.0;
    }
    let lv_lo = jian_core::estimate_at_site(ev.intensity_kind, ev.magnitude, ev.depth_km, d_lo).0;
    let lv_hi = jian_core::estimate_at_site(ev.intensity_kind, ev.magnitude, ev.depth_km, d_hi).0;

    if depth < INTENSITY_TRI_MAX_DEPTH && max_e > INTENSITY_TRI_SPLIT_KM && lv_lo != lv_hi {
        let m_ab = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
        let m_bc = [(b[0] + c[0]) * 0.5, (b[1] + c[1]) * 0.5];
        let m_ca = [(c[0] + a[0]) * 0.5, (c[1] + a[1]) * 0.5];
        let d = depth + 1;
        paint_intensity_tri_land(
            meshes, budget, view, lon_shift, max_edge_px, fill_alpha, ev, a, m_ab, m_ca, d,
        );
        paint_intensity_tri_land(
            meshes, budget, view, lon_shift, max_edge_px, fill_alpha, ev, m_ab, b, m_bc, d,
        );
        paint_intensity_tri_land(
            meshes, budget, view, lon_shift, max_edge_px, fill_alpha, ev, m_ca, m_bc, c, d,
        );
        paint_intensity_tri_land(
            meshes, budget, view, lon_shift, max_edge_px, fill_alpha, ev, m_ab, m_bc, m_ca, d,
        );
        return;
    }

    let mid = 0.5 * (d_lo + d_hi);
    let level =
        jian_core::estimate_at_site(ev.intensity_kind, ev.magnitude, ev.depth_km, mid).0;
    if level < 1 {
        return;
    }
    if emit_intensity_screen_tri(
        meshes, view, lon_shift, max_edge_px, fill_alpha, ev, level, a, b, c,
    ) {
        *budget = budget.saturating_sub(1);
    }
}

fn emit_intensity_screen_tri(
    meshes: &mut HashMap<u8, egui::epaint::Mesh>,
    view: &ViewCtx,
    lon_shift: f64,
    max_edge_px: f32,
    fill_alpha: u8,
    ev: &EewReport,
    level: u8,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
) -> bool {
    let project = |p: [f32; 2]| view.project_fill(p[0] as f64 + lon_shift, p[1] as f64);
    let (Some(pa), Some(pb), Some(pc)) = (project(a), project(b), project(c)) else {
        return false;
    };
    let max_e2 = max_edge_px * max_edge_px;
    if (pa - pb).length_sq() > max_e2
        || (pb - pc).length_sq() > max_e2
        || (pc - pa).length_sq() > max_e2
    {
        return false;
    }
    let rgb = color_for(ev.intensity_kind, level);
    let fill = Color32::from_rgba_unmultiplied(rgb.r, rgb.g, rgb.b, fill_alpha);
    let mesh = meshes.entry(level).or_default();
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(pa, fill);
    mesh.colored_vertex(pb, fill);
    mesh.colored_vertex(pc, fill);
    mesh.add_triangle(base, base + 1, base + 2);
    true
}

fn bbox_half_diag_km(ring: &RingSet) -> f64 {
    0.5 * haversine_km(
        ring.min_lon as f64,
        ring.min_lat as f64,
        ring.max_lon as f64,
        ring.max_lat as f64,
    )
}

fn point_in_bbox(lon: f64, lat: f64, ring: &RingSet) -> bool {
    lon >= ring.min_lon as f64
        && lon <= ring.max_lon as f64
        && lat >= ring.min_lat as f64
        && lat <= ring.max_lat as f64
}

fn point_in_ring(lon: f64, lat: f64, exterior: &[[f32; 2]]) -> bool {
    if exterior.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = exterior.len() - 1;
    for i in 0..exterior.len() {
        let (xi, yi) = (exterior[i][0] as f64, exterior[i][1] as f64);
        let (xj, yj) = (exterior[j][0] as f64, exterior[j][1] as f64);
        let intersect = ((yi > lat) != (yj > lat))
            && (lon < (xj - xi) * (lat - yi) / (yj - yi + 1e-15) + xi);
        if intersect {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn append_mesh(dest: &mut egui::epaint::Mesh, src: egui::epaint::Mesh) {
    let base = dest.vertices.len() as u32;
    dest.vertices.extend(src.vertices);
    dest.indices
        .extend(src.indices.into_iter().map(|i| i + base));
}

fn paint_cache_key(
    view: &ViewCtx,
    fill_land: bool,
    style: VectorStyle,
    gen: u64,
    skip_world_stroke: bool,
) -> u64 {
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
    skip_world_stroke.hash(&mut h);
    style.land_fill.to_array().hash(&mut h);
    style.world_border.to_array().hash(&mut h);
    style.admin_border.to_array().hash(&mut h);
    std::mem::discriminant(&view.mode).hash(&mut h);
    h.finish()
}


/// 对齐 GQ `GlobePanel`：scroll 分档绘制国界与区划（含藏南修复国界 / 中国一级行政区）。
fn paint_globe_gq_style(
    shapes: &mut Vec<egui::Shape>,
    basemap: &VectorBasemap,
    view: &ViewCtx,
    _fill_land: bool,
    style: VectorStyle,
) {
    // 球体固定只描轮廓（不做陆地填色 / 地平线弓弧），避免穿海弦线
    let (min_lon, min_lat, max_lon, max_lat) = (-180.0, -90.0, 180.0, 90.0);
    let stroke_w = Stroke::new(0.9_f32, style.world_border);
    let stroke_admin = Stroke::new(0.85_f32, style.admin_border);
    let scroll = zoom_to_scroll(view.zoom);

    if let Some(gq) = basemap.gq.as_ref() {
        if scroll >= 0.5 {
            paint_rings_globe(shapes, &gq.md, view, min_lon, min_lat, max_lon, max_lat, stroke_w);
        } else if scroll >= 0.25 {
            paint_rings_globe(
                shapes,
                &gq.hd_filtered,
                view,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                stroke_w,
            );
        } else {
            paint_rings_globe(
                shapes,
                &gq.uhd_filtered,
                view,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                stroke_w,
            );
        }
        if scroll < 0.5 {
            for rings in [&gq.cn, &gq.jp, &gq.us, &gq.ak, &gq.nz, &gq.hw] {
                if rings.is_empty() {
                    continue;
                }
                paint_rings_globe(
                    shapes,
                    rings,
                    view,
                    min_lon,
                    min_lat,
                    max_lon,
                    max_lat,
                    stroke_admin,
                );
            }
        }
        if scroll < 0.20 && !gq.it.is_empty() {
            paint_rings_globe(
                shapes,
                &gq.it,
                view,
                min_lon,
                min_lat,
                max_lon,
                max_lat,
                stroke_admin,
            );
        }
        return;
    }

    let rings = basemap.world.pick(view.zoom, 4.0, 6.0);
    paint_rings_globe(shapes, rings, view, min_lon, min_lat, max_lon, max_lat, stroke_w);
    if view.zoom >= ZOOM_REGIONAL && !basemap.regional.fine.is_empty() {
        let rings = basemap
            .regional
            .pick(view.zoom, ZOOM_REGIONAL_MED, ZOOM_REGIONAL_FINE);
        paint_rings_globe(
            shapes,
            rings,
            view,
            min_lon,
            min_lat,
            max_lon,
            max_lat,
            stroke_admin,
        );
    }
}

/// 球体：只描可见轮廓段（穿地平线断开，不闭合、不填色、不加弓弧）。
fn paint_rings_globe(
    shapes: &mut Vec<egui::Shape>,
    rings: &[RingSet],
    view: &ViewCtx,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    stroke: Stroke,
) {
    for ring in rings {
        if !ring_visible(ring, min_lon, min_lat, max_lon, max_lat) {
            continue;
        }
        for seg in project_globe_outline_segments(&ring.exterior, view) {
            if seg.len() < 2 {
                continue;
            }
            shapes.push(
                egui::epaint::PathShape {
                    points: seg,
                    closed: false,
                    fill: Color32::TRANSPARENT,
                    stroke: egui::epaint::PathStroke::new(stroke.width, stroke.color),
                }
                .into(),
            );
        }
    }
}

/// 将多边形外环按地平线可见性拆成多段折线；段内不做穿海连接。
fn project_globe_outline_segments(exterior: &[[f32; 2]], view: &ViewCtx) -> Vec<Vec<Pos2>> {
    if exterior.len() < 2 || !view.mode.is_globe() {
        return Vec::new();
    }
    let hr = match view.horizon_radius() {
        Some(r) => r,
        None => return Vec::new(),
    };
    let max_jump = (hr * 0.55).max(48.0);
    let n = exterior.len();
    let mut segs: Vec<Vec<Pos2>> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();

    // 多走一圈起点，闭合环在可见侧的连续段也能收齐
    for k in 0..=n {
        let i = k % n;
        let lon = exterior[i][0] as f64;
        let lat = exterior[i][1] as f64;
        match view.project(lon, lat) {
            Some(sp) => {
                if let Some(prev) = cur.last().copied() {
                    if (prev - sp).length() > max_jump {
                        if cur.len() >= 2 {
                            segs.push(std::mem::take(&mut cur));
                        } else {
                            cur.clear();
                        }
                    }
                }
                if cur.last().map(|p| (*p - sp).length() > 0.35).unwrap_or(true) {
                    cur.push(sp);
                }
            }
            None => {
                if cur.len() >= 2 {
                    segs.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
        if k == n {
            break;
        }
    }
    if cur.len() >= 2 {
        segs.push(cur);
    }
    segs
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
    // 先填后描。平面 / 球体均用屏幕 Path 实心填色（对齐 GQ）。
    if fill {
        for ring in rings {
            if !ring.may_fill() {
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
            if let Some(pts) =
                project_exterior_path_fill(ring, view, lon_shift, max_jump_px)
            {
                if pts.len() >= 3 {
                    shapes.push(egui::Shape::Path(egui::epaint::PathShape {
                        points: pts,
                        closed: true,
                        fill: land_fill,
                        stroke: egui::epaint::PathStroke::NONE,
                    }));
                }
            }
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


/// 外环 → 屏幕闭合路径（GQ：投影后再 fill）。孔洞忽略（与 GQ 只取外环一致）。
fn project_exterior_path_fill(
    ring: &RingSet,
    view: &ViewCtx,
    lon_shift: f64,
    max_jump_px: f32,
) -> Option<Vec<Pos2>> {
    if ring.exterior.len() < 3 {
        return None;
    }
    if view.mode.is_globe() {
        // 球体：全部顶点需落在地平线内，且相邻/首尾无大跳，否则跳过填色（只留描边）
        let mut pts = Vec::with_capacity(ring.exterior.len());
        let mut prev: Option<Pos2> = None;
        for p in &ring.exterior {
            let lon = p[0] as f64 + lon_shift;
            let lat = p[1] as f64;
            // 填色用放宽地平线的投影，避免大陆边缘被整环丢弃
            let Some(sp) = view.project_fill(lon, lat) else {
                return None;
            };
            if let Some(q) = prev {
                if (sp - q).length() > max_jump_px {
                    return None;
                }
            }
            pts.push(sp);
            prev = Some(sp);
        }
        if pts.len() < 3 {
            return None;
        }
        if (pts[0] - pts[pts.len() - 1]).length() > max_jump_px {
            return None;
        }
        // 周长异常扭曲才弃填；正常大陆轮廓放行
        let bb = Rect::from_points(&pts);
        let peri: f32 = pts
            .windows(2)
            .map(|w| (w[1] - w[0]).length())
            .sum::<f32>()
            + (pts[0] - pts[pts.len() - 1]).length();
        let diag = bb.width().hypot(bb.height()).max(1.0);
        if peri > diag * 14.0 {
            return None;
        }
        Some(pts)
    } else {
        project_ring_points_fill(&ring.exterior, view, lon_shift)
    }
}

/// 球体：外环+孔全部顶点须可投影；邻边过大则整环放弃。
/// 烈度区划：用经纬度预剖分 `ring.tris` + 屏幕投影顶点组 Mesh。
fn mesh_fill_intensity_ring(
    ring: &RingSet,
    view: &ViewCtx,
    lon_shift: f64,
    fill: Color32,
    max_edge_px: f32,
) -> Option<egui::epaint::Mesh> {
    if ring.tris.len() < 3 {
        return None;
    }
    let pts = if view.mode.is_globe() {
        project_fill_vertices_globe(ring, view, max_edge_px)?
    } else {
        project_fill_vertices(ring, view, lon_shift)?
    };
    let expected = ring.exterior.len()
        + ring
            .holes
            .iter()
            .filter(|h| h.len() >= 3)
            .map(|h| h.len())
            .sum::<usize>();
    if pts.len() != expected {
        return None;
    }
    let max_e2 = max_edge_px * max_edge_px;
    let mut mesh = egui::epaint::Mesh::default();
    mesh.reserve_vertices(pts.len());
    mesh.reserve_triangles(ring.tris.len() / 3);
    for p in &pts {
        mesh.colored_vertex(*p, fill);
    }
    let mut any = false;
    for tri in ring.tris.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= pts.len() || i1 >= pts.len() || i2 >= pts.len() {
            continue;
        }
        let a = pts[i0];
        let b = pts[i1];
        let c = pts[i2];
        if (a - b).length_sq() > max_e2
            || (b - c).length_sq() > max_e2
            || (c - a).length_sq() > max_e2
        {
            continue;
        }
        mesh.add_triangle(tri[0], tri[1], tri[2]);
        any = true;
    }
    if any {
        Some(mesh)
    } else {
        None
    }
}

fn project_fill_vertices_globe(
    ring: &RingSet,
    view: &ViewCtx,
    max_edge_px: f32,
) -> Option<Vec<Pos2>> {
    let mut out: Vec<Pos2> =
        Vec::with_capacity(ring.exterior.len() + ring.holes.iter().map(|h| h.len()).sum::<usize>());
    let push = |pts: &[[f32; 2]], out: &mut Vec<Pos2>| -> bool {
        let mut prev: Option<Pos2> = None;
        for p in pts {
            let Some(sp) = view.project_fill(p[0] as f64, p[1] as f64) else {
                return false;
            };
            if let Some(q) = prev {
                if (sp - q).length() > max_edge_px {
                    return false;
                }
            }
            out.push(sp);
            prev = Some(sp);
        }
        true
    };
    if !push(&ring.exterior, &mut out) {
        return None;
    }
    for hole in &ring.holes {
        if hole.len() < 3 {
            continue;
        }
        if !push(hole, &mut out) {
            return None;
        }
    }
    if out.len() < 3 {
        return None;
    }
    Some(out)
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

fn globalquake_dir(dir: &Path) -> PathBuf {
    dir.join("globalquake")
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

fn try_load_rings(path: &Path, files: &mut usize, skipped: &mut usize, label: &str) -> Option<Vec<RingSet>> {
    if !path.is_file() {
        return None;
    }
    *files += 1;
    match load_file(path) {
        Ok(rings) => {
            info!(
                path = %path.display(),
                rings = rings.len(),
                label,
                "loaded basemap layer"
            );
            Some(rings)
        }
        Err(e) => {
            *skipped += 1;
            warn!(path = %path.display(), error = %e, label, "底图加载失败");
            None
        }
    }
}

/// 加载 GlobalQuake Special 全部分层（国界 + 中日美新区划）。
fn try_load_gq_layers(dir: &Path) -> Option<(LayerLods, usize, usize, usize, GqLayers)> {
    let gq_dir = globalquake_dir(dir);
    if !gq_dir.is_dir() {
        return None;
    }
    let mut files = 0usize;
    let mut skipped = 0usize;
    let exclude_world = ["United States", "New Zealand", "Japan", "China"];
    let exclude_us = ["Alaska", "Hawaii"];

    let md = try_load_rings(
        &gq_dir.join("countriesMD.json"),
        &mut files,
        &mut skipped,
        "gq countriesMD",
    )?;
    let hd_filtered = load_file_excluding(&gq_dir.join("countriesHD.json"), &exclude_world)
        .map(|r| {
            files += 1;
            info!(rings = r.len(), "loaded gq countriesHD filtered");
            r
        })
        .unwrap_or_else(|e| {
            skipped += 1;
            warn!(error = %e, "gq HD filtered failed");
            Vec::new()
        });
    let uhd_filtered = load_file_excluding(&gq_dir.join("countriesUHD.json"), &exclude_world)
        .map(|r| {
            files += 1;
            info!(rings = r.len(), "loaded gq countriesUHD filtered");
            r
        })
        .unwrap_or_else(|e| {
            skipped += 1;
            warn!(error = %e, "gq UHD filtered failed");
            Vec::new()
        });

    let mut load_reg = |name: &str, excl: &[&str]| -> Vec<RingSet> {
        let p = gq_dir.join(name);
        if !p.is_file() {
            return Vec::new();
        }
        files += 1;
        match load_file_excluding(&p, excl) {
            Ok(r) => {
                info!(path = %p.display(), rings = r.len(), "loaded gq regional");
                r
            }
            Err(e) => {
                skipped += 1;
                warn!(path = %p.display(), error = %e, "gq regional failed");
                Vec::new()
            }
        }
    };

    let layers = GqLayers {
        md: md.clone(),
        hd_filtered,
        uhd_filtered,
        cn: load_reg("cn-prefectures.geojson", &[]),
        jp: load_reg("jp-prefectures.geojson", &[]),
        us: load_reg("us-albers.geojson", &exclude_us),
        ak: load_reg("AK-02-alaska-counties.geojson", &[]),
        nz: load_reg("new-zealand-districts.geojson", &[]),
        hw: load_reg("hawaii-countries.geojson", &[]),
        it: load_reg("italy_provinces.geojson", &[]),
    };

    let mut lods = LayerLods::from_fine(md, 0.35, 0.12);
    // 平面模式 coarse/medium/fine 仍可用 MD + 简化
    let diaoyu = builtin_diaoyu_patches();
    if !diaoyu.is_empty() {
        files += 1;
        lods.fine.extend(diaoyu.iter().cloned());
        lods.medium.extend(diaoyu.iter().cloned());
        lods.coarse.extend(diaoyu);
    }
    let rings = lods.fine.len();
    Some((lods, rings, files, skipped, layers))
}

/// 全球层 LOD：优先 GlobalQuake Special 全图层 → 回退。
fn load_world_lods(dir: &Path) -> (LayerLods, usize, usize, usize, Option<GqLayers>) {
    if let Some((lods, rings, files, skipped, gq)) = try_load_gq_layers(dir) {
        return (lods, rings, files, skipped, Some(gq));
    }
    let (world_fine, files, skipped) = load_world_rings_fallback(dir);
    let rings = world_fine.len();
    let lods = LayerLods::from_fine(world_fine, 0.35, 0.12);
    (lods, rings, files, skipped, None)
}

/// 非 GQ 回退：根目录精细 GeoJSON → 要石 Topo → Globalmap + `world/`。
fn load_world_rings_fallback(dir: &Path) -> (Vec<RingSet>, usize, usize) {
    let mut world_fine = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;

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

/// 世界层只叠钓鱼岛（点状）。藏南不另描边，由中国省界 / GQ 国界表达。
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

/// 区划层：中国 `geodata/省/*.json` 地市·区县扫盘 + 日本都道府县；
/// 不再优先 GQ 中国一级（整省一块，烈度填色过粗）。
fn load_regional_rings(dir: &Path) -> (Vec<RingSet>, usize, usize) {
    let mut regional_fine = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;

    let cn = load_cn_city_district_rings(dir, &mut files, &mut skipped);
    let cn_n = cn.len();
    regional_fine.extend(cn);

    let jp = load_jp_pref_rings(dir, &mut files, &mut skipped);
    let jp_n = jp.len();
    regional_fine.extend(jp);

    // 中国扫盘为空时回退 GQ / 要石省级，避免无填色
    if cn_n == 0 {
        let gq_cn = globalquake_dir(dir).join("cn-prefectures.geojson");
        if let Some(rings) = try_load_rings(
            &gq_cn,
            &mut files,
            &mut skipped,
            "fallback GlobalQuake cn-prefectures",
        ) {
            regional_fine.extend(rings);
        } else {
            let kdir = kanameishi_dir(dir);
            if let Some(path) = first_existing(
                &kdir,
                &[
                    "cn.province.topo.json",
                    "cn_province.geo.json",
                    "cn_province.geojson",
                ],
            ) {
                files += 1;
                match load_any_map(&path) {
                    Ok(rings) => {
                        info!(
                            path = %path.display(),
                            rings = rings.len(),
                            "fallback kanameishi cn province"
                        );
                        regional_fine.extend(rings);
                    }
                    Err(e) => {
                        skipped += 1;
                        warn!(path = %path.display(), error = %e, "要石中国省界加载失败");
                    }
                }
            }
        }
    }

    info!(
        cn_rings = cn_n,
        jp_rings = jp_n,
        total = regional_fine.len(),
        files,
        skipped,
        "regional admin rings loaded (city/district preferred)"
    );
    (regional_fine, files, skipped)
}

/// 远距 national 溶解源：GQ/要石一级区划（省 / 都道府县）。
fn load_coarse_admin_for_dissolve(dir: &Path) -> Vec<RingSet> {
    let mut out = Vec::new();
    let mut files = 0usize;
    let mut skipped = 0usize;
    let gq = globalquake_dir(dir);
    for name in ["cn-prefectures.geojson", "jp-prefectures.geojson"] {
        if let Some(rings) = try_load_rings(
            &gq.join(name),
            &mut files,
            &mut skipped,
            "coarse dissolve admin",
        ) {
            out.extend(rings);
        }
    }
    if !out.is_empty() {
        return out;
    }
    let kdir = kanameishi_dir(dir);
    for names in [
        [
            "cn.province.topo.json",
            "cn_province.geo.json",
            "cn_province.geojson",
        ],
        ["jp.pref.topo.json", "jp_pref.geo.json", "jp_pref.geojson"],
    ] {
        if let Some(path) = first_existing(&kdir, &names[..]) {
            if let Ok(rings) = load_any_map(&path) {
                out.extend(rings);
            }
        }
    }
    out
}

/// 扫描 `geodata/<省市区>/*.json`（及根目录台湾等），得到地市/区县环。
fn load_cn_city_district_rings(
    dir: &Path,
    files: &mut usize,
    skipped: &mut usize,
) -> Vec<RingSet> {
    let mut out = Vec::new();
    let globalmap = find_globalmap(dir);
    for entry in walk_geojson_skip_world(dir) {
        if globalmap.as_ref().is_some_and(|g| g == &entry) {
            continue;
        }
        if is_cn_boundary_patch(&entry) {
            continue;
        }
        if path_under_named_dir(&entry, &["kanameishi", "globalquake", "world", "日本"]) {
            continue;
        }
        *files += 1;
        match load_file(&entry) {
            Ok(rings) => out.extend(rings),
            Err(e) => {
                *skipped += 1;
                warn!(path = %entry.display(), error = %e, "跳过中国区划 GeoJSON");
            }
        }
    }
    if !out.is_empty() {
        info!(rings = out.len(), "loaded cn city/district basemap");
    }
    out
}

/// 日本都道府县：GQ → `日本/` → 要石。
fn load_jp_pref_rings(dir: &Path, files: &mut usize, skipped: &mut usize) -> Vec<RingSet> {
    let mut out = Vec::new();
    let gq_jp = globalquake_dir(dir).join("jp-prefectures.geojson");
    if let Some(rings) = try_load_rings(
        &gq_jp,
        files,
        skipped,
        "loaded GlobalQuake jp-prefectures",
    ) {
        return rings;
    }

    let jp_dir = dir.join("日本");
    if jp_dir.is_dir() {
        for entry in walk_geojson(&jp_dir) {
            *files += 1;
            match load_file(&entry) {
                Ok(rings) => out.extend(rings),
                Err(e) => {
                    *skipped += 1;
                    warn!(path = %entry.display(), error = %e, "跳过日本 GeoJSON");
                }
            }
        }
        if !out.is_empty() {
            info!(rings = out.len(), "loaded jp prefs from 日本/");
            return out;
        }
    }

    let kdir = kanameishi_dir(dir);
    if let Some(path) =
        first_existing(&kdir, &["jp.pref.topo.json", "jp_pref.geo.json", "jp_pref.geojson"])
    {
        *files += 1;
        match load_any_map(&path) {
            Ok(rings) => {
                info!(path = %path.display(), rings = rings.len(), "loaded kanameishi jp pref");
                return rings;
            }
            Err(e) => {
                *skipped += 1;
                warn!(path = %path.display(), error = %e, "要石日本区划加载失败");
            }
        }
    }
    out
}

fn path_under_named_dir(path: &Path, names: &[&str]) -> bool {
    path.components().any(|c| {
        let Some(n) = c.as_os_str().to_str() else {
            return false;
        };
        names.iter().any(|want| n.eq_ignore_ascii_case(want))
    })
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
    #[serde(default)]
    properties: Option<serde_json::Value>,
    geometry: Option<Geometry>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    geom_type: String,
    coordinates: serde_json::Value,
}

fn feature_name(props: &Option<serde_json::Value>) -> Option<String> {
    let props = props.as_ref()?;
    for key in [
        "name_long",
        "name",
        "NAME_2",
        "NAME_1",
        "NAME",
        "name_l",
        "admin",
    ] {
        if let Some(s) = props.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn load_file(path: &Path) -> Result<Vec<RingSet>, String> {
    load_file_excluding(path, &[])
}

/// 对齐 GQ `parseGeoJson(..., remove)`：按名称跳过要素（如近景过滤 China/Japan）。
fn load_file_excluding(path: &Path, exclude: &[&str]) -> Result<Vec<RingSet>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fc: FeatureCollection = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for feat in fc.features {
        if !exclude.is_empty() {
            if let Some(name) = feature_name(&feat.properties) {
                if exclude
                    .iter()
                    .any(|e| name.eq_ignore_ascii_case(e) || name == *e)
                {
                    continue;
                }
            }
        }
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
