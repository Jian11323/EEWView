//! 地图视口：GeoJSON 矢量底图（默认）/ 可选 XYZ 瓦片 + 走时表驱动 P/S 叠加层。

mod marker_icons;
mod overlay;
mod projection;
mod tile_cache;
mod topojson;
mod vector_basemap;

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

    /// 列表/图例联动：居中到经纬，可选改缩放
    pub fn center_on(&mut self, lon: f64, lat: f64, zoom: Option<f64>) {
        self.center_lon = projection::normalize_lon(lon);
        self.center_lat = projection::clamp_lat(lat);
        if let Some(z) = zoom {
            self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
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

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        active: Option<&EewReport>,
        travel: Option<&TravelEngine>,
        elapsed_s: f64,
        show_waves: bool,
        stations: &[StationSample],
        records: &[EqRecord],
        home: Option<(f64, f64)>,
    ) {
        if self.vector.poll() {
            ui.ctx().request_repaint();
        }
        self.markers.ensure(ui.ctx());

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

            // 速报散点已由 paint_records 绘制；此处叠加预警 X / 选中标签 / 波圈
            if show_waves {
                if let Some(ev) = active.filter(|e| !e.event_id.starts_with("station:")) {
                    overlay::paint_wave_overlay(
                        ui,
                        rect,
                        self.center_lon,
                        self.center_lat,
                        self.zoom,
                        Some(ev),
                        travel,
                        elapsed_s,
                        icons,
                    );
                }
            } else if let Some(ev) = active.filter(|e| !e.event_id.starts_with("station:")) {
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

        if resp.dragged() {
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

        let _ = Rect::from_min_size(rect.min, rect.size());
    }
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
