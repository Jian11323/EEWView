//! 震中点与 P/S 波阵面叠加。

use crate::projection;
use egui::{Color32, Rect, Stroke, Ui, Vec2};
use jian_core::EewReport;
use jian_travel::{TravelEngine, Wave};
use std::sync::Once;

/// 仅震中/测站点（无波圈）
pub fn paint_marker_only(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    ev: &EewReport,
) {
    let painter = ui.painter_at(rect);
    let p = projection::lonlat_to_screen(
        ev.longitude,
        ev.latitude,
        center_lon,
        center_lat,
        zoom,
        rect,
    );
    painter.circle_filled(p, 6.0, Color32::from_rgb(255, 200, 60));
    painter.circle_stroke(p, 6.0, Stroke::new(1.5_f32, Color32::WHITE));
    painter.text(
        p + Vec2::new(10.0, -14.0),
        egui::Align2::LEFT_BOTTOM,
        &ev.place,
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
}

pub fn paint_wave_overlay(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    active: Option<&EewReport>,
    travel: Option<&TravelEngine>,
    elapsed_s: f64,
) {
    let Some(ev) = active else { return };
    let painter = ui.painter_at(rect);

    let epicenter = projection::lonlat_to_screen(
        ev.longitude,
        ev.latitude,
        center_lon,
        center_lat,
        zoom,
        rect,
    );

    let (p_km, s_km) = if let Some(eng) = travel {
        (
            eng.surface_distance_for_elapsed(Wave::P, ev.depth_km, elapsed_s),
            eng.surface_distance_for_elapsed(Wave::S, ev.depth_km, elapsed_s),
        )
    } else {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| tracing::warn!("无走时表，波圈使用直线速度兜底"));
        (elapsed_s * 7.0, elapsed_s * 5.0)
    };

    let p_r = projection::km_to_pixels(p_km, ev.latitude, zoom).max(2.0);
    let s_r = projection::km_to_pixels(s_km, ev.latitude, zoom).max(2.0);

    let max_r = rect.width().max(rect.height()) * 2.0;
    if p_r < max_r {
        painter.circle_stroke(
            epicenter,
            p_r,
            Stroke::new(1.5_f32, Color32::from_rgb(80, 180, 255)),
        );
    }
    if s_r < max_r {
        painter.circle_stroke(
            epicenter,
            s_r,
            Stroke::new(2.0_f32, Color32::from_rgb(255, 80, 80)),
        );
    }

    painter.circle_filled(epicenter, 5.0, Color32::from_rgb(220, 40, 40));
    painter.circle_stroke(epicenter, 5.0, Stroke::new(1.0_f32, Color32::WHITE));
    painter.text(
        epicenter + Vec2::new(10.0, -18.0),
        egui::Align2::LEFT_BOTTOM,
        format!("M{:.1} {}", ev.magnitude, ev.place),
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
}
