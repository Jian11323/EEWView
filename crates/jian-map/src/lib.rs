//! 地图视口：GeoJSON 矢量底图（默认）/ 可选 XYZ 瓦片 + 走时表驱动 P/S 叠加层。

mod camera;
mod marker_icons;
mod overlay;
mod projection;
mod tile_cache;
mod topojson;
mod vector_basemap;

pub use camera::{fit_events, zoom_to_fit_radius_km, CameraFlight};
pub use projection::{clamp_lat, km_to_pixels, lonlat_to_screen, normalize_lon};
pub use tile_cache::TileCache;
pub use vector_basemap::{BasemapLoadState, BasemapMode, VectorBasemap, VectorStyle};

use egui::{Color32, Rect, Sense, Ui};
use jian_core::{EewReport, EqRecord, StationSample};
use jian_travel::TravelEngine;
use marker_icons::MarkerIcons;
use tile_cache::TileCache as Cache;
use vector_basemap::OCEAN;

const ZOOM_MIN: f64 = 2.0;
const ZOOM_MAX: f64 = 12.0;

pub struct MapViewport {
    pub center_lon: f64,
    pub center_lat: f64,
    pub zoom: f64,
    pub basemap: BasemapMode,
    ocean_fill: Color32,
    vector_style: VectorStyle,
    cache: Cache,
    vector: VectorBasemap,
    markers: MarkerIcons,
    flight: Option<CameraFlight>,
    /// 飞跃刚结束后抑制波圈自动缩放，避免落地瞬间被二次改 zoom「弹一下」
    post_flight_cooldown: f64,
    /// 用户拖拽/滚轮后暂停自动跟焦缩放，直到下一次主动飞跃
    pub user_camera_override: bool,
}

impl Default for MapViewport {
    fn default() -> Self {
        Self {
            center_lon: 135.0,
            center_lat: 35.0,
            zoom: 5.0,
            basemap: BasemapMode::Vector,
            ocean_fill: OCEAN,
            vector_style: VectorStyle::dark(),
            cache: Cache::new(
                "https://tilemap.sismotide.top".into(),
                "arcwob".into(),
            ),
            vector: VectorBasemap::default(),
            markers: MarkerIcons::default(),
            flight: None,
            post_flight_cooldown: 0.0,
            user_camera_override: false,
        }
    }
}

impl MapViewport {
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
        if basemap.show_vector() {
            if let Some(dir) = geodata_dir {
                self.vector.load_dir(dir);
            }
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
        self.post_flight_cooldown = 0.0;
        self.center_lon = projection::normalize_lon(lon);
        self.center_lat = projection::clamp_lat(lat);
        if let Some(z) = zoom {
            self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
        }
        self.user_camera_override = false;
    }

    /// 抛物线飞跃到目标震中（预警 / 速报列表切换与自动跟焦）。
    pub fn fly_to(&mut self, lon: f64, lat: f64, zoom: Option<f64>, duration_s: f64) {
        let end_zoom = zoom.unwrap_or(self.zoom).clamp(ZOOM_MIN, ZOOM_MAX);
        let dist = haversine_approx_deg(self.center_lon, self.center_lat, lon, lat);
        // 近距离瞬时跳转，避免无谓动画
        if dist < 0.05 && (self.zoom - end_zoom).abs() < 0.12 {
            self.center_on(lon, lat, Some(end_zoom));
            return;
        }
        self.flight = Some(CameraFlight::new(
            (self.center_lon, self.center_lat, self.zoom),
            (lon, lat, end_zoom),
            duration_s,
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
            // 落地后短暂冻结自动缩放，防止与波圈 fit 抢 zoom
            self.post_flight_cooldown = 0.85;
        }
        !done
    }

    pub fn is_flying(&self) -> bool {
        self.flight.is_some()
    }

    /// 飞跃中或落地冷却期内，波圈自动缩放应让路。
    pub fn blocks_auto_zoom(&self) -> bool {
        self.flight.is_some() || self.post_flight_cooldown > 0.0 || self.user_camera_override
    }

    /// 按波圈半径平滑跟缩放（不改中心）。
    pub fn apply_auto_zoom(&mut self, zoom: f64, smooth: f64) {
        if self.blocks_auto_zoom() {
            return;
        }
        let z = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let t = smooth.clamp(0.0, 1.0);
        let next = self.zoom + (z - self.zoom) * t;
        // 差距很小时直接贴齐，避免长期微抖
        self.zoom = if (z - next).abs() < 0.01 { z } else { next };
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
        stations: &[StationSample],
        records: &[EqRecord],
        home: Option<(f64, f64)>,
    ) -> Option<(f64, f64)> {
        if self.vector.poll() {
            ui.ctx().request_repaint();
        }
        self.markers.ensure(ui.ctx());

        let dt = ui.input(|i| i.stable_dt) as f64;
        if self.tick_flight(dt) || self.post_flight_cooldown > 0.0 {
            ui.ctx().request_repaint();
        }

        let (resp, _) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;

        ui.painter().rect_filled(rect, 0.0, self.ocean_fill);

        if self.basemap.show_tiles() {
            self.cache
                .paint(ui, rect, self.center_lon, self.center_lat, self.zoom);
        }

        if self.basemap.show_vector() {
            // vector 模式填陆地；叠在瓦片上时只描边界以免盖住影像
            let fill_land = self.basemap == BasemapMode::Vector;
            self.vector.paint(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                fill_land,
                self.vector_style,
            );
        }

        {
            let icons = Some(&self.markers);
            let selected_station = active.and_then(|e| e.event_id.strip_prefix("station:"));
            let selected_record = active.and_then(|e| {
                e.event_id
                    .strip_prefix("record:")
                    .map(|_| e.place.as_str())
            });
            overlay::paint_stations(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                stations,
                selected_station,
            );
            overlay::paint_records(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                records,
                selected_record,
                icons,
            );

            if let Some((hlon, hlat)) = home {
                overlay::paint_home(
                    ui,
                    rect,
                    self.center_lon,
                    self.center_lat,
                    self.zoom,
                    hlon,
                    hlat,
                    icons,
                );
            }

            let active_key = active.map(|e| (e.agency.0.as_str(), e.event_id.as_str()));
            // 各源最新预警叉标；传播中的事件随后由波圈层覆盖绘制
            overlay::paint_eew_markers(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                eew_markers,
                active_key,
                icons,
            );

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
                        rect,
                        self.center_lon,
                        self.center_lat,
                        self.zoom,
                        Some(ev),
                        travel,
                        elapsed,
                        icons,
                        selected,
                    );
                }
                if !painted_active {
                    if let Some(ev) = active.filter(|e| {
                        !e.event_id.starts_with("station:") && !e.event_id.starts_with("record:")
                    }) {
                        overlay::paint_wave_overlay(
                            ui,
                            rect,
                            self.center_lon,
                            self.center_lat,
                            self.zoom,
                            Some(ev),
                            travel,
                            event_elapsed_s(ev, now_ms),
                            icons,
                            true,
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
                    overlay::paint_marker_only(
                        ui,
                        rect,
                        self.center_lon,
                        self.center_lat,
                        self.zoom,
                        ev,
                        icons,
                    );
                }
            }
        }

        if resp.dragged() {
            self.flight = None;
            self.post_flight_cooldown = 0.0;
            self.user_camera_override = true;
            let d = resp.drag_delta();
            let (lon, lat) = projection::pan_by_screen_delta(
                self.center_lon,
                self.center_lat,
                self.zoom,
                d.x,
                d.y,
            );
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
                        let (lon, lat) = screen_to_lonlat(
                            pos,
                            rect,
                            self.center_lon,
                            self.center_lat,
                            before,
                        );
                        self.zoom = after;
                        let dx = (pos.x - rect.center().x) as f64;
                        let dy = (pos.y - rect.center().y) as f64;
                        let (wx, wy) = projection::lonlat_to_world(lon, lat, after);
                        let (nlon, nlat) =
                            projection::world_to_lonlat(wx - dx, wy - dy, after);
                        self.center_lon = projection::normalize_lon(nlon);
                        self.center_lat = projection::clamp_lat(nlat);
                    } else {
                        self.zoom = after;
                    }
                }
            }
        }

        let mut picked = None;
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let (lon, lat) =
                    screen_to_lonlat(pos, rect, self.center_lon, self.center_lat, self.zoom);
                picked = Some((
                    projection::normalize_lon(lon),
                    projection::clamp_lat(lat),
                ));
            }
        }
        picked
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

fn screen_to_lonlat(
    pos: egui::Pos2,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
) -> (f64, f64) {
    let (cx, cy) = projection::lonlat_to_world(center_lon, center_lat, zoom);
    let wx = cx + (pos.x - rect.center().x) as f64;
    let wy = cy + (pos.y - rect.center().y) as f64;
    projection::world_to_lonlat(wx, wy, zoom)
}

fn event_elapsed_s(ev: &EewReport, now_ms: i64) -> f64 {
    if ev.origin_ms > 0 && now_ms > 0 {
        ((now_ms - ev.origin_ms) as f64 / 1000.0).max(0.0)
    } else {
        0.0
    }
}
