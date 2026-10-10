//! 震中点、测站点与 P/S 波阵面叠加。

use crate::marker_icons::MarkerIcons;
use crate::projection;
use egui::{Color32, Pos2, Rect, Stroke, TextureHandle, Ui, Vec2};
use jian_core::palette::color_for;
use jian_core::{EewReport, EqRecord, StationSample};
use jian_travel::{max_s_wave_spread_km, TravelEngine, Wave};
use std::sync::Once;

const MARKER_RED: Color32 = Color32::from_rgb(220, 40, 40);
const HOME_FALLBACK: Color32 = Color32::from_rgb(80, 220, 80);

/// 速报 SVG 边长（随震级）
fn mag_icon_size(magnitude: f64) -> f32 {
    (12.0 + magnitude * 3.2).clamp(14.0, 42.0) as f32
}

fn paint_tex(painter: &egui::Painter, tex: &TextureHandle, p: Pos2, size: f32, tint: Color32) {
    let rect = Rect::from_center_size(p, Vec2::splat(size));
    painter.image(
        tex.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
}

fn paint_red_x_fallback(painter: &egui::Painter, p: Pos2, half: f32) {
    let white = Stroke::new(half * 0.55 + 1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    let red = Stroke::new(half * 0.45 + 1.6, MARKER_RED);
    let a = p + Vec2::new(-half, -half);
    let b = p + Vec2::new(half, half);
    let c = p + Vec2::new(half, -half);
    let d = p + Vec2::new(-half, half);
    painter.line_segment([a, b], white);
    painter.line_segment([c, d], white);
    painter.line_segment([a, b], red);
    painter.line_segment([c, d], red);
}

fn paint_red_dot_fallback(painter: &egui::Painter, p: Pos2, r: f32, selected: bool) {
    painter.circle_filled(p, r, MARKER_RED);
    painter.circle_stroke(
        p,
        r,
        Stroke::new(
            if selected { 1.6_f32 } else { 1.0_f32 },
            if selected {
                Color32::WHITE
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 160)
            },
        ),
    );
}

fn paint_eew_marker(painter: &egui::Painter, icons: Option<&MarkerIcons>, p: Pos2) {
    if let Some(tex) = icons.and_then(|i| i.cross()) {
        paint_tex(painter, tex, p, 26.0, Color32::WHITE);
    } else {
        paint_red_x_fallback(painter, p, 7.0);
    }
}

fn paint_record_marker(
    painter: &egui::Painter,
    icons: Option<&MarkerIcons>,
    p: Pos2,
    magnitude: f64,
    selected: bool,
) {
    let size = mag_icon_size(magnitude) * if selected { 1.12 } else { 1.0 };
    if let Some(tex) = icons.and_then(|i| i.red()) {
        let tint = if selected {
            Color32::WHITE
        } else {
            Color32::from_rgba_unmultiplied(255, 255, 255, 220)
        };
        paint_tex(painter, tex, p, size, tint);
    } else {
        paint_red_dot_fallback(painter, p, size * 0.35, selected);
    }
}

fn paint_home_marker(painter: &egui::Painter, icons: Option<&MarkerIcons>, p: Pos2) {
    if let Some(tex) = icons.and_then(|i| i.green()) {
        paint_tex(painter, tex, p, 24.0, Color32::WHITE);
    } else {
        painter.circle_stroke(p, 7.0, Stroke::new(3.0_f32, Color32::WHITE));
        painter.circle_stroke(p, 7.0, Stroke::new(1.8_f32, HOME_FALLBACK));
    }
}

fn is_record_event(ev: &EewReport) -> bool {
    ev.event_id.starts_with("record:")
}

fn paint_epicenter_marker(
    painter: &egui::Painter,
    icons: Option<&MarkerIcons>,
    p: Pos2,
    ev: &EewReport,
    with_label: bool,
) {
    if is_record_event(ev) {
        paint_record_marker(painter, icons, p, ev.magnitude, true);
    } else {
        paint_eew_marker(painter, icons, p);
    }
    if with_label {
        painter.text(
            p + Vec2::new(12.0, -20.0),
            egui::Align2::LEFT_BOTTOM,
            format!("M{:.1} {}", ev.magnitude, ev.place),
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
    }
}

/// 有感测站散点（视口外跳过）
pub fn paint_stations(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    stations: &[StationSample],
    selected_id: Option<&str>,
) {
    if stations.is_empty() {
        return;
    }
    let painter = ui.painter_at(rect);
    let pad = 8.0_f32;
    for s in stations {
        let p = projection::lonlat_to_screen(
            s.longitude,
            s.latitude,
            center_lon,
            center_lat,
            zoom,
            rect,
        );
        if p.x < rect.left() - pad
            || p.x > rect.right() + pad
            || p.y < rect.top() - pad
            || p.y > rect.bottom() + pad
        {
            continue;
        }
        let rgb = color_for(s.intensity_kind, s.intensity_level);
        let fill = Color32::from_rgb(rgb.r, rgb.g, rgb.b);
        let selected = selected_id == Some(s.id.as_str());
        let r = if selected { 5.5 } else { 3.0 };
        painter.circle_filled(p, r, fill);
        painter.circle_stroke(
            p,
            r,
            Stroke::new(
                if selected { 1.5_f32 } else { 0.8_f32 },
                if selected {
                    Color32::WHITE
                } else {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 140)
                },
            ),
        );
        if selected {
            painter.text(
                p + Vec2::new(8.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{} {}", s.name, s.intensity_text),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }
}

/// 速报列表：`red.svg`，尺寸随震级
pub fn paint_records(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    records: &[EqRecord],
    selected_place: Option<&str>,
    icons: Option<&MarkerIcons>,
) {
    if records.is_empty() {
        return;
    }
    let painter = ui.painter_at(rect);
    let pad = 24.0_f32;
    for r in records.iter().rev() {
        let p = projection::lonlat_to_screen(
            r.longitude,
            r.latitude,
            center_lon,
            center_lat,
            zoom,
            rect,
        );
        if p.x < rect.left() - pad
            || p.x > rect.right() + pad
            || p.y < rect.top() - pad
            || p.y > rect.bottom() + pad
        {
            continue;
        }
        let selected = selected_place == Some(r.place.as_str());
        paint_record_marker(&painter, icons, p, r.magnitude, selected);
    }
}

/// 本机位置（`green.svg`）
pub fn paint_home(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    lon: f64,
    lat: f64,
    icons: Option<&MarkerIcons>,
) {
    let painter = ui.painter_at(rect);
    let p = projection::lonlat_to_screen(lon, lat, center_lon, center_lat, zoom, rect);
    if !rect.expand(20.0).contains(p) {
        return;
    }
    paint_home_marker(&painter, icons, p);
}

/// 仅震中标记（无波圈）
pub fn paint_marker_only(
    ui: &mut Ui,
    rect: Rect,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    ev: &EewReport,
    icons: Option<&MarkerIcons>,
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
    paint_epicenter_marker(&painter, icons, p, ev, true);
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
    icons: Option<&MarkerIcons>,
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

    let max_km = max_s_wave_spread_km(ev.magnitude);
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

    let max_r = rect.width().max(rect.height()) * 2.0;
    if p_km < max_km {
        let p_r = projection::km_to_pixels(p_km, ev.latitude, zoom).max(2.0);
        if p_r < max_r {
            painter.circle_stroke(
                epicenter,
                p_r,
                Stroke::new(1.5_f32, Color32::from_rgb(80, 180, 255)),
            );
        }
    }
    if s_km < max_km {
        let s_r = projection::km_to_pixels(s_km, ev.latitude, zoom).max(2.0);
        if s_r < max_r {
            painter.circle_stroke(
                epicenter,
                s_r,
                Stroke::new(2.0_f32, Color32::from_rgb(255, 80, 80)),
            );
        }
    }

    paint_epicenter_marker(&painter, icons, epicenter, ev, true);
}
