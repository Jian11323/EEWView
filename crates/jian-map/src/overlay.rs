//! 震中点、测站点与 P/S 波阵面叠加。

use crate::marker_icons::MarkerIcons;
use crate::projection::{self, ViewCtx};
use egui::{Color32, Pos2, Rect, Stroke, TextureHandle, Ui, Vec2};
use jian_core::palette::color_for;
use jian_core::{EewReport, EqRecord, StationSample, VolcanoNotice};
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


fn felt_level(s: &StationSample) -> u8 {
    if s.intensity_kind == jian_core::IntensityKind::JmaShindo {
        if s.intensity_level >= 1 {
            return s.intensity_level.min(9);
        }
        if s.instrumental >= 0.5 {
            return jian_core::jma_from_instrumental(s.instrumental).0;
        }
    }
    0
}

/// 测站散点：震度 >=1 用 `shindo_inst`；其余計測色点。有预警时有感点始终显示。
pub fn paint_stations(
    ui: &mut Ui,
    view: &ViewCtx,
    stations: &[StationSample],
    selected_id: Option<&str>,
    icons: Option<&MarkerIcons>,
    eew_active: bool,
) {
    if stations.is_empty() {
        return;
    }
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let pad = 16.0_f32;
    // 低缩放隐藏静默点；中缩放抽稀静默点以减负
    let show_quiet = view.zoom >= 4.5;
    let quiet_stride = if view.zoom < 5.5 {
        4usize
    } else if view.zoom < 6.5 {
        2
    } else {
        1
    };
    let _ = eew_active;
    // 先静默点、后有感图标，且有感按震度升序，避免震度1被蓝点盖住
    let mut order: Vec<usize> = (0..stations.len()).collect();
    order.sort_by(|&a, &b| {
        let sa = &stations[a];
        let sb = &stations[b];
        let la = felt_level(sa);
        let lb = felt_level(sb);
        la.cmp(&lb).then_with(|| {
            sa.instrumental
                .partial_cmp(&sb.instrumental)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let mut quiet_i = 0usize;
    for idx in order {
        let s = &stations[idx];
        let felt = s.intensity_level >= 1
            || (s.intensity_kind == jian_core::IntensityKind::JmaShindo && s.instrumental >= 0.5);
        let quiet = !felt && s.intensity_level == 0;
        if quiet {
            if !show_quiet {
                if selected_id != Some(s.id.as_str()) {
                    continue;
                }
            } else if selected_id != Some(s.id.as_str()) {
                quiet_i += 1;
                if quiet_stride > 1 && (quiet_i % quiet_stride) != 0 {
                    continue;
                }
            }
        }
        let Some(p) = view.project(s.longitude, s.latitude) else {
            continue;
        };
        if p.x < rect.left() - pad
            || p.x > rect.right() + pad
            || p.y < rect.top() - pad
            || p.y > rect.bottom() + pad
        {
            continue;
        }
        let selected = selected_id == Some(s.id.as_str());
        let jma_lv = if s.intensity_kind == jian_core::IntensityKind::JmaShindo {
            if s.intensity_level >= 1 {
                s.intensity_level.min(9)
            } else if s.instrumental >= 0.5 {
                jian_core::jma_from_instrumental(s.instrumental).0
            } else {
                0
            }
        } else {
            0
        };
        if jma_lv >= 1 {
            if let Some(tex) = icons.and_then(|i| i.shindo_inst(jma_lv)) {
                // 震度1略小；高等级略大；选中再放大
                let base = match jma_lv {
                    1 => 12.0,
                    2 => 14.0,
                    3 | 4 => 16.0,
                    5 | 6 => 18.0,
                    _ => 20.0,
                };
                let size = if selected { base + 4.0 } else { base };
                paint_tex(&painter, tex, p, size, Color32::WHITE);
                if selected {
                    painter.text(
                        p + Vec2::new(size * 0.55, -size * 0.45),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{} {}", s.name, jian_core::jma_shindo_text(jma_lv)),
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                }
                continue;
            }
        }
        let rgb = if s.instrumental >= -3.0 {
            jian_core::instrumental_rgb(s.instrumental)
        } else {
            color_for(s.intensity_kind, s.intensity_level)
        };
        let fill = Color32::from_rgb(rgb.r, rgb.g, rgb.b);
        let r = if selected {
            5.5
        } else if quiet {
            2.2
        } else {
            3.2
        };
        painter.circle_filled(p, r, fill);
        // 选中才描边；常态无白边，避免测站密网发糊发脏
        if selected {
            painter.circle_stroke(p, r, Stroke::new(1.5_f32, Color32::WHITE));
        }
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



/// 火山：按经纬度画实心三角 ▲
pub fn paint_volcanoes(
    ui: &mut Ui,
    view: &ViewCtx,
    volcanoes: &[VolcanoNotice],
    selected_idx: Option<usize>,
) {
    if volcanoes.is_empty() {
        return;
    }
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let pad = 20.0_f32;
    let fill = Color32::from_rgb(0xF9, 0x73, 0x16);
    let fill_sel = Color32::from_rgb(0xFF, 0xB0, 0x40);
    for (i, v) in volcanoes.iter().enumerate() {
        if v.latitude.abs() <= 0.01 && v.longitude.abs() <= 0.01 {
            continue;
        }
        let Some(p) = view.project(v.longitude, v.latitude) else {
            continue;
        };
        if p.x < rect.left() - pad
            || p.x > rect.right() + pad
            || p.y < rect.top() - pad
            || p.y > rect.bottom() + pad
        {
            continue;
        }
        let selected = selected_idx == Some(i);
        let h = if selected { 14.0_f32 } else { 10.0_f32 };
        let half = h * 0.72;
        let pts = vec![
            egui::pos2(p.x, p.y - h),
            egui::pos2(p.x - half, p.y + h * 0.55),
            egui::pos2(p.x + half, p.y + h * 0.55),
        ];
        painter.add(egui::Shape::convex_polygon(
            pts,
            if selected { fill_sel } else { fill },
            Stroke::new(
                if selected { 1.6_f32 } else { 1.0_f32 },
                Color32::from_rgb(0x33, 0x1A, 0x00),
            ),
        ));
        if selected {
            painter.text(
                p + Vec2::new(h * 0.7, -h),
                egui::Align2::LEFT_BOTTOM,
                &v.name,
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }
}

/// 速报列表：`markers/red.svg`，尺寸随震级
pub fn paint_records(
    ui: &mut Ui,
    view: &ViewCtx,
    records: &[EqRecord],
    selected_place: Option<&str>,
    icons: Option<&MarkerIcons>,
) {
    if records.is_empty() {
        return;
    }
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let pad = 24.0_f32;
    for r in records.iter().rev() {
        let Some(p) = view.project(r.longitude, r.latitude) else {
            continue;
        };
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

/// 本机位置（`markers/green.svg` 蓝房子）
pub fn paint_home(
    ui: &mut Ui,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    icons: Option<&MarkerIcons>,
) {
    let Some(p) = view.project(lon, lat) else {
        return;
    };
    if !view.rect.expand(20.0).contains(p) {
        return;
    }
    let painter = ui.painter_at(view.rect);
    paint_home_marker(&painter, icons, p);
}

/// 仅震中标记（无波圈）
pub fn paint_marker_only(
    ui: &mut Ui,
    view: &ViewCtx,
    ev: &EewReport,
    icons: Option<&MarkerIcons>,
) {
    let Some(p) = view.project(ev.longitude, ev.latitude) else {
        return;
    };
    let painter = ui.painter_at(view.rect);
    paint_epicenter_marker(&painter, icons, p, ev, true);
}

fn stroke_polyline(painter: &egui::Painter, segs: &[Vec<Pos2>], stroke: Stroke) {
    for pts in segs {
        if pts.len() < 2 {
            continue;
        }
        painter.add(egui::Shape::line(pts.clone(), stroke));
    }
}

pub fn paint_wave_overlay(
    ui: &mut Ui,
    view: &ViewCtx,
    active: Option<&EewReport>,
    travel: Option<&TravelEngine>,
    elapsed_s: f64,
    icons: Option<&MarkerIcons>,
    selected: bool,
) {
    let Some(ev) = active else { return };
    let rect = view.rect;
    let painter = ui.painter_at(rect);

    let Some(epicenter) = view.project(ev.longitude, ev.latitude) else {
        return;
    };

    let max_km = max_s_wave_spread_km(ev.magnitude);
    let (p_km, s_km) = if let Some(eng) = travel {
        (
            eng.surface_distance_for_elapsed(Wave::P, ev.depth_km, elapsed_s),
            eng.surface_distance_for_elapsed(Wave::S, ev.depth_km, elapsed_s),
        )
    } else {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| tracing::warn!("无走时表，波圈使用直线速度兜底"));
        let depth = ev.depth_km.max(0.0);
        let surface = |v: f64| {
            let slant = elapsed_s * v;
            if slant <= depth {
                0.0
            } else {
                (slant * slant - depth * depth).sqrt()
            }
        };
        (surface(7.0), surface(5.0))
    };

    let (p_alpha, s_alpha, p_w, s_w) = if selected {
        (255_u8, 255_u8, 3.2_f32, 4.0_f32)
    } else {
        (170, 180, 2.4, 3.0)
    };

    let max_r = rect.width().max(rect.height()) * 3.0;
    if s_km < max_km {
        let fade = (1.0 - (s_km / max_km).clamp(0.0, 1.0)).mul_add(0.35, 0.65);
        let p_stroke = Stroke::new(
            p_w,
            Color32::from_rgba_unmultiplied(255, 255, 255, ((p_alpha as f64) * fade) as u8),
        );
        let s_rgb = color_for(ev.intensity_kind, ev.intensity_level);
        let s_stroke = Stroke::new(
            s_w,
            Color32::from_rgba_unmultiplied(
                s_rgb.r,
                s_rgb.g,
                s_rgb.b,
                ((s_alpha as f64) * fade) as u8,
            ),
        );
        if view.mode.is_globe() {
            stroke_polyline(
                &painter,
                &view.circle_segments(ev.longitude, ev.latitude, p_km),
                p_stroke,
            );
            if s_km > 0.5 {
                stroke_polyline(
                    &painter,
                    &view.circle_segments(ev.longitude, ev.latitude, s_km),
                    s_stroke,
                );
            }
        } else {
            let p_r = projection::km_to_pixels(p_km, ev.latitude, view.zoom).max(3.0);
            if p_r < max_r {
                painter.circle_stroke(epicenter, p_r, p_stroke);
            }
            let s_r = projection::km_to_pixels(s_km, ev.latitude, view.zoom).max(3.0);
            if s_r < max_r && s_km > 0.5 {
                painter.circle_stroke(epicenter, s_r, s_stroke);
            }
        }
    }

    paint_epicenter_marker(&painter, icons, epicenter, ev, selected);
}

/// 各数据源最新预警叉标（无波圈；选中项带标签）
pub fn paint_eew_markers(
    ui: &mut Ui,
    view: &ViewCtx,
    markers: &[EewReport],
    selected: Option<(&str, &str)>,
    icons: Option<&MarkerIcons>,
) {
    if markers.is_empty() {
        return;
    }
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let pad = 28.0_f32;
    let multi = markers.len() > 1;
    for (i, ev) in markers.iter().enumerate() {
        let Some(p) = view.project(ev.longitude, ev.latitude) else {
            continue;
        };
        if p.x < rect.left() - pad
            || p.x > rect.right() + pad
            || p.y < rect.top() - pad
            || p.y > rect.bottom() + pad
        {
            continue;
        }
        let is_sel = selected == Some((ev.agency.0.as_str(), ev.event_id.as_str()));
        paint_epicenter_marker(&painter, icons, p, ev, is_sel);
        if multi {
            let label = format!("{}", i + 1);
            let pos = egui::pos2(p.x + 14.0, p.y - 18.0);
            painter.text(
                pos,
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(16.0),
                Color32::WHITE,
            );
        }
    }
}
