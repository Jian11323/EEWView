//! 地图视口：GeoJSON 矢量底图（默认）/ 可选 XYZ 瓦片 + 走时表驱动 P/S 叠加层。

mod camera;
mod detect;
mod fault_lines;
mod globe;
mod marker_icons;
mod overlay;
mod projection;
mod tile_cache;
mod topojson;
mod tsunami_coast;
mod vector_basemap;

pub use camera::{
    cluster_for_fit, fit_events, globe_framed_center, zoom_to_fit_radius_km, CameraFlight,
};
pub use detect::{DetectBox, DetectTracker};
pub use overlay::TyphoonHoverHit;
pub use projection::{clamp_lat, km_to_pixels, lonlat_to_screen, normalize_lon, ProjectionMode};
pub use tile_cache::TileCache;
pub use vector_basemap::{BasemapLoadState, BasemapMode, VectorBasemap, VectorStyle, OCEAN};

/// 绘制质量：随性能档位降低测站密度 / 矢量细度，减轻老 GPU/CPU。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PaintQuality {
    Eco,
    #[default]
    Normal,
    Extreme,
}

impl PaintQuality {
    pub fn from_profile(profile: &str) -> Self {
        match profile.to_ascii_lowercase().as_str() {
            "eco" => Self::Eco,
            "extreme" => Self::Extreme,
            _ => Self::Normal,
        }
    }
}

/// 地图一帧交互结果。
#[derive(Debug, Default, Clone)]
pub struct MapUiResult {
    pub click: Option<(f64, f64)>,
    pub typhoon_hover: Option<TyphoonHoverHit>,
}

use egui::{Color32, Sense, Stroke, Ui};
use jian_core::{EewReport, EqRecord, StationSample, TsunamiInfo, TyphoonInfo, VolcanoNotice};
use jian_travel::TravelEngine;
use marker_icons::MarkerIcons;
use projection::ViewCtx;
use fault_lines::FaultLines;
use tsunami_coast::TsunamiCoast;
use tile_cache::TileCache as Cache;

const ZOOM_MIN: f64 = 2.0;
const ZOOM_MAX: f64 = 12.0;

pub struct MapViewport {
    pub center_lon: f64,
    pub center_lat: f64,
    pub zoom: f64,
    pub basemap: BasemapMode,
    pub projection_mode: ProjectionMode,
    ocean_fill: Color32,
    vector_style: VectorStyle,
    cache: Cache,
    vector: VectorBasemap,
    tsunami_coast: TsunamiCoast,
    faults: FaultLines,
    markers: MarkerIcons,
    flight: Option<CameraFlight>,
    /// 飞跃刚结束后抑制波圈自动缩放，避免落地瞬间被二次改 zoom「弹一下」
    post_flight_cooldown: f64,
    /// 用户拖拽/滚轮后暂停自动跟焦缩放，直到下一次主动飞跃
    pub user_camera_override: bool,
    /// 最近一帧视口短边（px），供跟焦时按波圈估 zoom
    pub last_viewport_min_px: f32,
    /// 性能档对应的绘制抽稀
    paint_quality: PaintQuality,
}

impl Default for MapViewport {
    fn default() -> Self {
        Self {
            center_lon: 135.0,
            center_lat: 35.0,
            zoom: 6.8,
            basemap: BasemapMode::Vector,
            projection_mode: ProjectionMode::Flat,
            ocean_fill: OCEAN,
            vector_style: VectorStyle::dark(),
            cache: Cache::new(
                "https://tilemap.sismotide.top".into(),
                "arcwob".into(),
            ),
            vector: VectorBasemap::default(),
            tsunami_coast: TsunamiCoast::default(),
            faults: FaultLines::default(),
            markers: MarkerIcons::default(),
            flight: None,
            post_flight_cooldown: 0.0,
            user_camera_override: false,
            last_viewport_min_px: 720.0,
            paint_quality: PaintQuality::Normal,
        }
    }
}

impl MapViewport {
    pub fn set_projection_mode(&mut self, mode: ProjectionMode) {
        self.projection_mode = mode;
    }

    pub fn configure(
        &mut self,
        basemap: BasemapMode,
        tile_base: impl Into<String>,
        tile_source: impl Into<String>,
        geodata_dir: Option<&std::path::Path>,
    ) {
        self.basemap = basemap;
        self.cache
            .set_source(tile_base.into(), tile_source.into());
        if let Some(dir) = geodata_dir {
            // 球体 / 矢量底图：加载国界与区划
            if basemap.show_vector() || self.projection_mode.is_globe() {
                self.vector.load_dir(dir);
            }
            // 海啸沿岸与底图模式无关，始终尝试加载
            self.tsunami_coast.load_dir(dir);
            self.faults.load_dir(dir);
        }
    }

    pub fn configure_tiles(&mut self, tile_base: impl Into<String>, tile_source: impl Into<String>) {
        self.cache
            .set_source(tile_base.into(), tile_source.into());
    }

    /// 一键清除内存瓦片缓存。
    pub fn clear_tile_cache(&mut self) {
        self.cache.clear();
    }

    /// 列表/图例联动：瞬时居中到经纬，可选改缩放
    pub fn center_on(&mut self, lon: f64, lat: f64, zoom: Option<f64>) {
        self.flight = None;
        self.post_flight_cooldown = 1.2;
        self.center_lon = projection::normalize_lon(lon);
        self.center_lat = projection::clamp_lat(lat);
        if let Some(z) = zoom {
            self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
        }
        self.user_camera_override = false;
    }

    /// GQ 风格跳转：`smoothTransition` 抛物线飞到目标；近距则瞬时到位。
    /// 球体远距目标会做太平洋侧构图偏置。
    pub fn fly_to(&mut self, lon: f64, lat: f64, zoom: Option<f64>, duration_s: f64) {
        self.fly_to_inner(lon, lat, zoom, duration_s, true);
    }

    /// 列表/卡片点选：镜头对准震中经纬，不做球体构图偏置。
    pub fn fly_to_centered(&mut self, lon: f64, lat: f64, zoom: Option<f64>, duration_s: f64) {
        self.fly_to_inner(lon, lat, zoom, duration_s, false);
    }

    fn fly_to_inner(
        &mut self,
        lon: f64,
        lat: f64,
        zoom: Option<f64>,
        duration_s: f64,
        frame_globe: bool,
    ) {
        let end_zoom = zoom.unwrap_or(self.zoom).clamp(ZOOM_MIN, ZOOM_MAX);
        let (lon, lat) = if frame_globe && self.projection_mode.is_globe() {
            globe_framed_center(lon, lat, end_zoom)
        } else {
            (lon, lat)
        };
        let dist = haversine_approx_deg(self.center_lon, self.center_lat, lon, lat);
        if dist < 0.05 && (self.zoom - end_zoom).abs() < 0.12 {
            self.center_on(lon, lat, Some(end_zoom));
            return;
        }
        // GQ runAnimation 约 5s；平面图按大圆距离略缩短
        let km = dist * 111.0;
        let dur = (duration_s.max(1.2) + km / 9000.0).clamp(1.2, 4.5);
        self.flight = Some(CameraFlight::new(
            (self.center_lon, self.center_lat, self.zoom),
            (lon, lat, end_zoom),
            dur,
        ));
        self.post_flight_cooldown = 0.0;
        self.user_camera_override = false;
    }

    /// 推进镜头动画；返回是否仍在飞跃中。
    pub fn tick_flight(&mut self, dt: f64) -> bool {
        if self.post_flight_cooldown > 0.0 {
            self.post_flight_cooldown = (self.post_flight_cooldown - dt).max(0.0);
        }
        let Some(flight) = self.flight.as_mut() else {
            return false;
        };
        let (lon, lat, zoom, done) = flight.tick(dt);
        self.center_lon = lon;
        self.center_lat = lat;
        self.zoom = zoom;
        if done {
            let (elon, elat, ez) = flight.target();
            self.center_lon = elon;
            self.center_lat = elat;
            self.zoom = ez;
            self.flight = None;
            // 抑制落地瞬间波圈 auto_zoom 二次改 zoom（弹簧感）
            self.post_flight_cooldown = 2.8;
        }
        !done
    }

    pub fn is_flying(&self) -> bool {
        self.flight.is_some()
    }

    /// 预警期跟 S 波：只线性缩小（zoom 减小），不放大、不停顿跳级。
    /// 球体远距时按震中做太平洋侧构图偏置，减轻「左俄多、右美缺」。
    pub fn apply_linear_zoom_out(
        &mut self,
        target_zoom: f64,
        rate: f64,
        focus_lon: Option<f64>,
        focus_lat: Option<f64>,
    ) {
        if self.flight.is_some() || self.user_camera_override || self.post_flight_cooldown > 0.0 {
            return;
        }
        let target = target_zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        if target >= self.zoom - 0.02 && focus_lon.is_none() {
            return;
        }
        let k = rate.clamp(0.02, 0.35);
        if target < self.zoom - 0.02 {
            self.zoom = (self.zoom + (target - self.zoom) * k).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        if self.projection_mode.is_globe() {
            if let (Some(flon), Some(flat)) = (focus_lon, focus_lat) {
                let (tlon, tlat) = globe_framed_center(flon, flat, self.zoom);
                let dlon = projection::normalize_lon(tlon - self.center_lon);
                let dlon = if dlon > 180.0 {
                    dlon - 360.0
                } else if dlon < -180.0 {
                    dlon + 360.0
                } else {
                    dlon
                };
                let dlat = tlat - self.center_lat;
                if dlon.abs() > 0.05 || dlat.abs() > 0.05 {
                    let kb = (k * 0.85).clamp(0.02, 0.3);
                    self.center_lon = projection::normalize_lon(self.center_lon + dlon * kb);
                    self.center_lat = projection::clamp_lat(self.center_lat + dlat * kb);
                }
            }
        }
    }

    pub fn load_state(&self) -> BasemapLoadState {
        self.vector.load_state()
    }

    pub fn load_status(&self) -> &str {
        self.vector.status_text()
    }

    /// 与界面主题同步的海洋/缺瓦片底色。
    pub fn set_ocean_fill(&mut self, color: Color32) {
        self.ocean_fill = color;
    }

    /// 与界面主题同步的矢量陆地/边界色。
    pub fn set_vector_style(&mut self, style: VectorStyle) {
        self.vector_style = style;
    }

    /// 与性能档位同步的绘制抽稀（节能更狠）。
    pub fn set_paint_quality(&mut self, q: PaintQuality) {
        self.paint_quality = q;
    }

    /// 绘制地图交互；若单击（非拖拽）则返回点击处经纬度。
    /// `wave_events`：仍在传播的多震；`eew_markers`：各源最新预警叉标；`active` 高亮选中。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        active: Option<&EewReport>,
        wave_events: &[EewReport],
        eew_markers: &[EewReport],
        travel: Option<&TravelEngine>,
        now_ms: i64,
        show_waves: bool,
        show_intensity_rings: bool,
        stations: &[StationSample],
        records: &[EqRecord],
        volcanoes: &[VolcanoNotice],
        selected_volcano: Option<usize>,
        typhoons: &[TyphoonInfo],
        typhoon_forecast_agency: &str,
        home: Option<(f64, f64)>,
        tsunami: Option<&TsunamiInfo>,
        detect_boxes: &[DetectBox],
        show_faults: bool,
    ) -> MapUiResult {
        if self.vector.poll() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.markers.ensure(ui.ctx());

        let dt = ui.input(|i| i.stable_dt) as f64;
        if self.tick_flight(dt) || self.post_flight_cooldown > 0.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }

        let (resp, _) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.last_viewport_min_px = rect.width().min(rect.height()).max(120.0);

        let view = ViewCtx::new(
            self.projection_mode,
            self.center_lon,
            self.center_lat,
            self.zoom,
            rect,
        );
        let globe = self.projection_mode.is_globe();

        ui.painter().rect_filled(rect, 0.0, self.ocean_fill);

        if globe {
            if let Some(r) = view.horizon_radius() {
                // 球体外遮罩：地平线外用略深色，勾出球轮廓
                ui.painter().circle_filled(rect.center(), r, self.ocean_fill);
                ui.painter().circle_stroke(
                    rect.center(),
                    r,
                    Stroke::new(1.2_f32, Color32::from_rgb(0x55, 0x58, 0x60)),
                );
            }
        }

        // 平面瓦片（tiles/both）
        if !globe && self.basemap.show_tiles() {
            self.cache
                .paint(ui, rect, self.center_lon, self.center_lat, self.zoom);
        }

        // 烈度衰减：优先陆地三角分带（贴区划，类 Hikari 锯齿外缘）；无区划时回退同心环。
        // `show_intensity_rings`：由设置控制；速报另需右键预览标志。
        if show_intensity_rings {
            let ring_ev = active.filter(|e| {
                !e.is_cancel && !e.event_id.starts_with("station:")
            });
            if let Some(ev) = ring_ev {
                if !self.vector.paint_intensity_regions(ui, &view, ev, true) {
                    overlay::paint_intensity_rings(ui, &view, ev, true);
                }
            }
        }

        // 暗色矢量 / 球体：只描轮廓，不填陆地（对齐旧版纯黑轮廓图）
        if globe || self.basemap.show_vector() {
            self.vector.paint(
                ui,
                &view,
                /* fill_land */ false,
                self.vector_style,
                /* skip_world_stroke */ false,
                self.paint_quality,
            );
        }

        // 活动断层：矢量轮廓之上、事件叠加之下；瓦片底图时也绘制
        if show_faults {
            let dark = self.ocean_fill.r() < 80;
            self.faults.paint(ui, &view, dark, self.paint_quality);
        }

        // JMA 海啸：按沿岸预报区描边闪烁（不对中日外轮廓整圈描边）
        if let Some(t) = tsunami {
            if !t.cancelled && !t.grade.eq_ignore_ascii_case("Cancel") {
                self.tsunami_coast.paint(ui, &view, &t.areas_list, now_ms);
            }
        }

        let mut typhoon_hover = None;
        {
            let icons = Some(&self.markers);
            let selected_station = active.and_then(|e| e.event_id.strip_prefix("station:"));
            let selected_record = active.and_then(|e| {
                e.event_id
                    .strip_prefix("record:")
                    .map(|_| e.place.as_str())
            });
            let eew_active = active.is_some_and(|e| {
                !e.event_id.starts_with("station:")
                    && !e.event_id.starts_with("record:")
                    && !e.is_cancel
            });
            detect::paint_detect_boxes(ui, &view, detect_boxes);
            overlay::paint_stations(
                ui,
                &view,
                stations,
                selected_station,
                icons,
                eew_active,
                self.paint_quality,
            );
            overlay::paint_records(ui, &view, records, selected_record, icons);
            overlay::paint_volcanoes(ui, &view, volcanoes, selected_volcano);
            if !typhoons.is_empty() {
                let hover = resp.hover_pos();
                typhoon_hover = overlay::paint_typhoons(
                    ui,
                    &view,
                    typhoons,
                    typhoon_forecast_agency,
                    hover,
                );
            }

            if let Some((hlon, hlat)) = home {
                overlay::paint_home(ui, &view, hlon, hlat, icons);
            }

            let active_key = active.map(|e| (e.agency.0.as_str(), e.event_id.as_str()));
            // 有传播中波圈时叉标 500ms 闪烁（对齐 eewcn eewMarksTimer）
            let blink_active = wave_events.iter().any(|e| {
                !e.event_id.starts_with("station:") && !e.event_id.starts_with("record:")
            });
            let show_cross = !blink_active || overlay::eew_mark_blink_on(now_ms);

            overlay::paint_eew_markers(ui, &view, eew_markers, active_key, icons, show_cross);

            if show_waves {
                let mut painted_active = false;
                for ev in wave_events {
                    if ev.event_id.starts_with("station:") || ev.event_id.starts_with("record:") {
                        continue;
                    }
                    let elapsed = event_elapsed_s(ev, now_ms);
                    let selected = active_key
                        == Some((ev.agency.0.as_str(), ev.event_id.as_str()));
                    if selected {
                        painted_active = true;
                    }
                    overlay::paint_wave_overlay(
                        ui,
                        &view,
                        Some(ev),
                        travel,
                        elapsed,
                        icons,
                        selected,
                        show_cross,
                    );
                }
                if !painted_active {
                    if let Some(ev) = active.filter(|e| {
                        !e.event_id.starts_with("station:") && !e.event_id.starts_with("record:")
                    }) {
                        overlay::paint_wave_overlay(
                            ui,
                            &view,
                            Some(ev),
                            travel,
                            event_elapsed_s(ev, now_ms),
                            icons,
                            true,
                            show_cross,
                        );
                    }
                }
            } else if let Some(ev) = active.filter(|e| {
                !e.event_id.starts_with("station:") && !e.event_id.starts_with("record:")
            }) {
                let already = eew_markers.iter().any(|m| {
                    m.agency.0 == ev.agency.0 && m.event_id == ev.event_id
                });
                if !already {
                    overlay::paint_marker_only(ui, &view, ev, icons);
                }
            }
        }

        if resp.dragged() {
            self.flight = None;
            self.post_flight_cooldown = 0.0;
            self.user_camera_override = true;
            let d = resp.drag_delta();
            let (lon, lat) = view.pan_by_delta(d.x, d.y);
            self.center_lon = lon;
            self.center_lat = lat;
        }

        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.0 {
                self.flight = None;
                self.post_flight_cooldown = 0.0;
                self.user_camera_override = true;
                let before = self.zoom;
                let after = (before + scroll as f64 * 0.002).clamp(ZOOM_MIN, ZOOM_MAX);
                if (after - before).abs() > 1e-9 {
                    if let Some(pos) = resp.hover_pos() {
                        if globe {
                            // 球体：缩放只改相机高度，中心经纬不变
                            let _ = pos;
                            self.zoom = after;
                        } else {
                            let (lon, lat) = view.screen_to_lonlat(pos);
                            self.zoom = after;
                            let dx = (pos.x - rect.center().x) as f64;
                            let dy = (pos.y - rect.center().y) as f64;
                            let (wx, wy) = projection::lonlat_to_world(lon, lat, after);
                            let (nlon, nlat) =
                                projection::world_to_lonlat(wx - dx, wy - dy, after);
                            self.center_lon = projection::normalize_lon(nlon);
                            self.center_lat = projection::clamp_lat(nlat);
                        }
                    } else {
                        self.zoom = after;
                    }
                }
            }
        }

        let mut click = None;
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (lon, lat) = view.screen_to_lonlat(pos);
                click = Some((
                    projection::normalize_lon(lon),
                    projection::clamp_lat(lat),
                ));
            }
        }
        MapUiResult {
            click,
            typhoon_hover,
        }
    }
}

/// 粗略角距离（度），用于判断是否值得开飞跃动画。
fn haversine_approx_deg(lon0: f64, lat0: f64, lon1: f64, lat1: f64) -> f64 {
    let mut dlon = projection::normalize_lon(lon1 - lon0).abs();
    if dlon > 180.0 {
        dlon = 360.0 - dlon;
    }
    let dlat = (lat1 - lat0).abs();
    (dlon * dlon + dlat * dlat).sqrt()
}

fn event_elapsed_s(ev: &EewReport, now_ms: i64) -> f64 {
    if ev.origin_ms > 0 && now_ms > 0 {
        ((now_ms - ev.origin_ms) as f64 / 1000.0).max(0.0)
    } else {
        0.0
    }
}
