//! 地图视口：Web Mercator 瓦片 + 走时表驱动 P/S 叠加层。

mod overlay;
mod projection;
mod tile_cache;

pub use projection::{clamp_lat, km_to_pixels, lonlat_to_screen};
pub use tile_cache::TileCache;

use egui::{Color32, Rect, Sense, Ui};
use jian_core::EewReport;
use jian_travel::TravelEngine;
use tile_cache::TileCache as Cache;

const ZOOM_MIN: f64 = 2.0;
const ZOOM_MAX: f64 = 12.0;

pub struct MapViewport {
    pub center_lon: f64,
    pub center_lat: f64,
    pub zoom: f64,
    cache: Cache,
}

impl Default for MapViewport {
    fn default() -> Self {
        Self {
            center_lon: 135.0,
            center_lat: 35.0,
            zoom: 5.0,
            cache: Cache::new(
                "https://tilemap.sismotide.top".into(),
                "arcwob".into(),
            ),
        }
    }
}

impl MapViewport {
    pub fn configure_tiles(&mut self, tile_base: impl Into<String>, tile_source: impl Into<String>) {
        self.cache
            .set_source(tile_base.into(), tile_source.into());
    }

    /// 列表/图例联动：居中到经纬，可选改缩放
    pub fn center_on(&mut self, lon: f64, lat: f64, zoom: Option<f64>) {
        self.center_lon = lon;
        self.center_lat = projection::clamp_lat(lat);
        if let Some(z) = zoom {
            self.zoom = z.clamp(ZOOM_MIN, ZOOM_MAX);
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        active: Option<&EewReport>,
        travel: Option<&TravelEngine>,
        elapsed_s: f64,
        show_waves: bool,
    ) {
        let (resp, _) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;

        // 底色（瓦片未到时）
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(12, 14, 18));

        self.cache
            .paint(ui, rect, self.center_lon, self.center_lat, self.zoom);

        if show_waves {
            overlay::paint_wave_overlay(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                active,
                travel,
                elapsed_s,
            );
        } else if let Some(ev) = active {
            overlay::paint_marker_only(
                ui,
                rect,
                self.center_lon,
                self.center_lat,
                self.zoom,
                ev,
            );
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
                        self.center_lon = nlon;
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
