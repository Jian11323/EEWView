//! 震中点、测站点与 P/S 波阵面叠加。

use crate::globe;
use crate::marker_icons::MarkerIcons;
use crate::projection::{self, ViewCtx};
use egui::epaint::{Mesh, Vertex};
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

/// 平面：轴对齐 billboard；球体：贴面方块（对齐 GQ `createSquare` + `project3D`）。
/// 球缘角点穿地平线时不回退 billboard（否则会再次「立起」），改画小圆点。
fn paint_tex(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    tex: &TextureHandle,
    p: Pos2,
    size: f32,
    tint: Color32,
) {
    if view.mode.is_globe() {
        if let Some([tl, tr, br, bl]) = view.project_surface_square(lon, lat, size) {
            let mut mesh = Mesh::with_texture(tex.id());
            mesh.vertices.extend_from_slice(&[
                Vertex {
                    pos: tl,
                    uv: Pos2::new(0.0, 0.0),
                    color: tint,
                },
                Vertex {
                    pos: tr,
                    uv: Pos2::new(1.0, 0.0),
                    color: tint,
                },
                Vertex {
                    pos: br,
                    uv: Pos2::new(1.0, 1.0),
                    color: tint,
                },
                Vertex {
                    pos: bl,
                    uv: Pos2::new(0.0, 1.0),
                    color: tint,
                },
            ]);
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
        // 角点穿地平线：不回退 billboard（会立起），与 GQ shouldDraw=false 一致
        return;
    }
    let rect = Rect::from_center_size(p, Vec2::splat(size));
    painter.image(
        tex.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        tint,
    );
}

fn paint_red_x_fallback(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    p: Pos2,
    half_px: f32,
) {
    let white = Stroke::new(half_px * 0.55 + 1.2, Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    let red = Stroke::new(half_px * 0.45 + 1.6, MARKER_RED);
    // 球体：大圆十字（对齐 GQ `createCross`）；平面：屏幕对角叉
    if view.mode.is_globe() {
        if let Some(r_km) = view.px_to_km(half_px * 2.0) {
            let mut arms: Vec<(Pos2, Pos2)> = Vec::with_capacity(2);
            for br0 in [45.0_f64, 135.0] {
                let (a_lon, a_lat) = globe::move_on_globe(lon, lat, r_km, br0);
                let (b_lon, b_lat) = globe::move_on_globe(lon, lat, r_km, br0 + 180.0);
                if let (Some(a), Some(b)) = (view.project(a_lon, a_lat), view.project(b_lon, b_lat)) {
                    arms.push((a, b));
                }
            }
            if arms.len() == 2 {
                for &(a, b) in &arms {
                    painter.line_segment([a, b], white);
                }
                for &(a, b) in &arms {
                    painter.line_segment([a, b], red);
                }
                return;
            }
        }
    }
    let a = p + Vec2::new(-half_px, -half_px);
    let b = p + Vec2::new(half_px, half_px);
    let c = p + Vec2::new(half_px, -half_px);
    let d = p + Vec2::new(-half_px, half_px);
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

/// 预警震中叉标闪烁相位（对齐 eewcn `eewMarksTimer` 500ms）。
pub fn eew_mark_blink_on(now_ms: i64) -> bool {
    if now_ms <= 0 {
        return true;
    }
    (now_ms / 500) % 2 == 0
}

fn paint_eew_marker(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    icons: Option<&MarkerIcons>,
    p: Pos2,
) {
    if let Some(tex) = icons.and_then(|i| i.cross()) {
        paint_tex(painter, view, lon, lat, tex, p, 26.0, Color32::WHITE);
    } else {
        paint_red_x_fallback(painter, view, lon, lat, p, 7.0);
    }
}

fn paint_record_marker(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
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
        paint_tex(painter, view, lon, lat, tex, p, size, tint);
    } else {
        paint_red_dot_fallback(painter, p, size * 0.35, selected);
    }
}

fn paint_home_marker(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    icons: Option<&MarkerIcons>,
    p: Pos2,
) {
    if let Some(tex) = icons.and_then(|i| i.green()) {
        paint_tex(painter, view, lon, lat, tex, p, 24.0, Color32::WHITE);
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
    view: &ViewCtx,
    icons: Option<&MarkerIcons>,
    p: Pos2,
    ev: &EewReport,
    with_label: bool,
    show_cross: bool,
) {
    if is_record_event(ev) {
        paint_record_marker(
            painter,
            view,
            ev.longitude,
            ev.latitude,
            icons,
            p,
            ev.magnitude,
            true,
        );
    } else if show_cross {
        // 有波圈时按相位显隐，实现红色叉标闪烁
        paint_eew_marker(painter, view, ev.longitude, ev.latitude, icons, p);
    }
    if with_label && show_cross {
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
    match s.intensity_kind {
        jian_core::IntensityKind::JmaShindo => {
            if s.intensity_level >= 1 {
                return s.intensity_level.min(9);
            }
            if s.instrumental >= 0.5 {
                return jian_core::jma_from_instrumental(s.instrumental).0;
            }
            0
        }
        jian_core::IntensityKind::CnIntensity => {
            if s.intensity_level >= 1 {
                s.intensity_level.min(12)
            } else {
                0
            }
        }
    }
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
        let felt = match s.intensity_kind {
            jian_core::IntensityKind::JmaShindo => {
                s.intensity_level >= 1 || s.instrumental >= 0.5
            }
            jian_core::IntensityKind::CnIntensity => s.intensity_level >= 1,
        };
        let quiet = !felt;
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
                paint_tex(
                    &painter,
                    view,
                    s.longitude,
                    s.latitude,
                    tex,
                    p,
                    size,
                    Color32::WHITE,
                );
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
        // 烈度网有感：csis_inst 图标（与侧栏一致）
        if s.intensity_kind == jian_core::IntensityKind::CnIntensity && s.intensity_level >= 1 {
            let cn_lv = s.intensity_level.clamp(1, 12);
            if let Some(tex) = icons.and_then(|i| i.csis_inst(cn_lv)) {
                let base = match cn_lv {
                    1..=3 => 12.0,
                    4..=6 => 15.0,
                    7..=9 => 18.0,
                    _ => 20.0,
                };
                let size = if selected { base + 4.0 } else { base };
                paint_tex(
                    &painter,
                    view,
                    s.longitude,
                    s.latitude,
                    tex,
                    p,
                    size,
                    Color32::WHITE,
                );
                if selected {
                    painter.text(
                        p + Vec2::new(size * 0.55, -size * 0.45),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{} {}", s.name, s.intensity_text),
                        egui::FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                }
                continue;
            }
        }
        // 回退色点：烈度用 CN 色；JMA 計測用色带
        let rgb = match s.intensity_kind {
            jian_core::IntensityKind::CnIntensity => {
                if s.intensity_level >= 1 {
                    color_for(s.intensity_kind, s.intensity_level)
                } else {
                    jian_core::palette::Rgb::new(0x66, 0x66, 0x66)
                }
            }
            jian_core::IntensityKind::JmaShindo => {
                if s.instrumental >= -3.0 {
                    jian_core::instrumental_rgb(s.instrumental)
                } else {
                    color_for(s.intensity_kind, s.intensity_level)
                }
            }
        };
        let fill = Color32::from_rgb(rgb.r, rgb.g, rgb.b);
        let r = if selected {
            5.5
        } else if s.intensity_kind == jian_core::IntensityKind::CnIntensity && felt {
            match s.intensity_level {
                1..=3 => 3.6,
                4..=6 => 4.4,
                _ => 5.2,
            }
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
        paint_record_marker(
            &painter,
            view,
            r.longitude,
            r.latitude,
            icons,
            p,
            r.magnitude,
            selected,
        );
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
    paint_home_marker(&painter, view, lon, lat, icons, p);
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
    paint_epicenter_marker(&painter, view, icons, p, ev, true, true);
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
    show_cross: bool,
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

    paint_epicenter_marker(
        &painter,
        view,
        icons,
        epicenter,
        ev,
        selected,
        show_cross,
    );
}

/// 各数据源最新预警叉标（无波圈；选中项带标签）。
/// `show_cross`：有传播中波圈时按闪烁相位传入，对齐 eewcn。
pub fn paint_eew_markers(
    ui: &mut Ui,
    view: &ViewCtx,
    markers: &[EewReport],
    selected: Option<(&str, &str)>,
    icons: Option<&MarkerIcons>,
    show_cross: bool,
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
        paint_epicenter_marker(&painter, view, icons, p, ev, is_sel, show_cross);
        if multi && show_cross {
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

/// 台风悬停命中：中心或路径点，用于绘制实况详情。
#[derive(Debug, Clone)]
pub struct TyphoonHoverHit {
    pub anchor: Pos2,
    pub id: String,
    pub name: String,
    pub strong: String,
    pub power: i32,
    pub time_text: String,
    pub longitude: f64,
    pub latitude: f64,
    pub speed_ms: f64,
    pub pressure_hpa: f64,
    pub move_direction: String,
    pub move_speed_kmh: f64,
    /// 是否中心（否则为历史路径点）
    #[allow(dead_code)]
    pub is_center: bool,
}

/// 台风：实况路径、预报路径、风圈、中心徽章与悬停详情（对齐 eewcn MapTyphoonLogic）。
pub fn paint_typhoons(
    ui: &mut Ui,
    view: &ViewCtx,
    typhoons: &[jian_core::TyphoonInfo],
    forecast_agency: &str,
    hover_pos: Option<Pos2>,
) -> Option<TyphoonHoverHit> {
    if typhoons.is_empty() {
        return None;
    }
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let agency_sel = forecast_agency.trim();
    let show_all = agency_sel.is_empty() || agency_sel == "全部";

    let mut best_hit: Option<(f32, TyphoonHoverHit)> = None;
    let hit_r2 = 14.0_f32 * 14.0;

    for tf in typhoons {
        let last = tf.points.last();
        // 风圈（7 / 10 / 12 级）— 先画，避免盖住路径
        if let Some(pt) = last {
            paint_wind_circles(
                &painter,
                view,
                pt.longitude,
                pt.latitude,
                pt.radius7,
                pt.radius10,
                pt.radius12,
            );
        }

        // 实况路径折线（段色按终点等级）
        for w in tf.points.windows(2) {
            let a = &w[0];
            let b = &w[1];
            let (r, g, bl) = jian_core::typhoon_color_rgb(&b.strong, b.power);
            let stroke = Stroke::new(2.4_f32, Color32::from_rgb(r, g, bl));
            paint_geodesic_segment(
                &painter,
                view,
                a.longitude,
                a.latitude,
                b.longitude,
                b.latitude,
                stroke,
            );
        }

        // 预报路径
        if let Some(pt) = last {
            for fc in &pt.forecasts {
                if !show_all {
                    let norm = normalize_typhoon_agency(&fc.agency);
                    if norm != agency_sel && fc.agency != agency_sel {
                        continue;
                    }
                }
                if fc.points.len() < 2 {
                    continue;
                }
                let (r, g, bl) = jian_core::typhoon_forecast_agency_rgb(&fc.agency);
                let stroke = Stroke::new(
                    2.0_f32,
                    Color32::from_rgba_unmultiplied(r, g, bl, 200),
                );
                for w in fc.points.windows(2) {
                    paint_geodesic_segment(
                        &painter,
                        view,
                        w[0].longitude,
                        w[0].latitude,
                        w[1].longitude,
                        w[1].latitude,
                        stroke,
                    );
                }
                if !show_all {
                    break;
                }
            }
        }

        // 历史点 + 悬停
        let n = tf.points.len();
        for (i, pt) in tf.points.iter().enumerate() {
            let is_latest = i + 1 == n;
            let Some(p) = project_near(view, pt.longitude, pt.latitude) else {
                continue;
            };
            if !rect.expand(12.0).contains(p) {
                continue;
            }
            if !is_latest {
                let (r, g, bl) = jian_core::typhoon_color_rgb(&pt.strong, pt.power);
                let rad = match jian_core::typhoon_grade_level(&pt.strong, pt.power) {
                    0 => 7.0,
                    1 | 2 | 3 => 6.0,
                    4 => 5.0,
                    _ => 4.0,
                };
                painter.circle_filled(p, rad, Color32::from_rgb(r, g, bl));
                painter.circle_stroke(p, rad, Stroke::new(1.0_f32, Color32::WHITE));
            }
            if let Some(hp) = hover_pos {
                let d2 = (hp.x - p.x).powi(2) + (hp.y - p.y).powi(2);
                if d2 <= hit_r2 {
                    let hit = TyphoonHoverHit {
                        anchor: p,
                        id: tf.id.clone(),
                        name: tf.name.clone(),
                        strong: if pt.strong.is_empty() {
                            tf.strong.clone()
                        } else {
                            pt.strong.clone()
                        },
                        power: if pt.power > 0 { pt.power } else { tf.power },
                        time_text: pt.time_text.clone(),
                        longitude: pt.longitude,
                        latitude: pt.latitude,
                        speed_ms: if pt.speed_ms > 0.0 {
                            pt.speed_ms
                        } else {
                            tf.wind_speed_ms
                        },
                        pressure_hpa: if pt.pressure_hpa > 0.0 {
                            pt.pressure_hpa
                        } else {
                            tf.pressure_hpa
                        },
                        move_direction: if pt.move_direction.is_empty() {
                            tf.move_direction.clone()
                        } else {
                            pt.move_direction.clone()
                        },
                        move_speed_kmh: if pt.move_speed_kmh > 0.0 {
                            pt.move_speed_kmh
                        } else {
                            tf.move_speed_kmh
                        },
                        is_center: is_latest,
                    };
                    if best_hit.as_ref().is_none_or(|(bd, _)| d2 < *bd) {
                        best_hit = Some((d2, hit));
                    }
                }
            }
        }

        // 当前中心 + 徽章
        let Some(p) = project_near(view, tf.longitude, tf.latitude) else {
            continue;
        };
        if !rect.expand(80.0).contains(p) {
            continue;
        }
        let (r, g, bl) = jian_core::typhoon_color_rgb(&tf.strong, tf.power);
        painter.circle_stroke(p, 9.0, Stroke::new(2.2_f32, Color32::WHITE));
        painter.circle_stroke(p, 6.5, Stroke::new(2.0_f32, Color32::from_rgb(r, g, bl)));

        let time = last
            .map(|x| format_typhoon_time(&x.time_text))
            .unwrap_or_default();
        let strong = if tf.strong.is_empty() {
            last.map(|x| x.strong.as_str()).unwrap_or("")
        } else {
            tf.strong.as_str()
        };
        let badge_hover = paint_typhoon_badge(
            &painter,
            p,
            strong,
            &tf.name,
            &time,
            Color32::from_rgb(r, g, bl),
        );

        if let Some(hp) = hover_pos {
            let d2 = (hp.x - p.x).powi(2) + (hp.y - p.y).powi(2);
            let on_badge = badge_hover.contains(hp);
            if d2 <= hit_r2 || on_badge {
                let hit = TyphoonHoverHit {
                    anchor: p,
                    id: tf.id.clone(),
                    name: tf.name.clone(),
                    strong: strong.to_string(),
                    power: tf.power,
                    time_text: last.map(|x| x.time_text.clone()).unwrap_or_default(),
                    longitude: tf.longitude,
                    latitude: tf.latitude,
                    speed_ms: last
                        .map(|x| x.speed_ms)
                        .filter(|&v| v > 0.0)
                        .unwrap_or(tf.wind_speed_ms),
                    pressure_hpa: last
                        .map(|x| x.pressure_hpa)
                        .filter(|&v| v > 0.0)
                        .unwrap_or(tf.pressure_hpa),
                    move_direction: last
                        .map(|x| x.move_direction.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| tf.move_direction.clone()),
                    move_speed_kmh: last
                        .map(|x| x.move_speed_kmh)
                        .filter(|&v| v > 0.0)
                        .unwrap_or(tf.move_speed_kmh),
                    is_center: true,
                };
                let score = if on_badge { 0.0 } else { d2 };
                if best_hit.as_ref().is_none_or(|(bd, _)| score < *bd) {
                    best_hit = Some((score, hit));
                }
            }
        }
    }

    if let Some((_, hit)) = best_hit {
        paint_typhoon_detail(&painter, &hit);
        Some(hit)
    } else {
        None
    }
}

fn project_near(view: &ViewCtx, lon: f64, lat: f64) -> Option<Pos2> {
    let lon = projection::wrap_lon_near(lon, view.center_lon);
    view.project(lon, lat)
}

fn paint_typhoon_badge(
    painter: &egui::Painter,
    center: Pos2,
    strong: &str,
    name: &str,
    time: &str,
    grade_color: Color32,
) -> Rect {
    let font = egui::FontId::proportional(13.0);
    let type_gal = painter.layout_no_wrap(
        if strong.is_empty() {
            "台风".into()
        } else {
            strong.to_string()
        },
        font.clone(),
        Color32::WHITE,
    );
    let name_txt = if time.is_empty() {
        name.to_string()
    } else {
        format!("{name}  {time}")
    };
    let name_gal = painter.layout_no_wrap(name_txt, font, Color32::WHITE);
    let pad_x = 8.0_f32;
    let pad_y = 4.0_f32;
    let h = type_gal.size().y.max(name_gal.size().y) + pad_y * 2.0;
    let type_w = type_gal.size().x + pad_x * 2.0;
    let name_w = name_gal.size().x + pad_x * 2.0;
    let origin = egui::pos2(center.x + 12.0, center.y - h * 0.5);
    let type_rect = Rect::from_min_size(origin, Vec2::new(type_w, h));
    let name_rect = Rect::from_min_size(
        egui::pos2(origin.x + type_w, origin.y),
        Vec2::new(name_w, h),
    );
    let full = type_rect.union(name_rect);
    painter.rect_filled(type_rect, 3.0, grade_color);
    painter.rect_stroke(
        type_rect,
        3.0,
        Stroke::new(1.0_f32, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
    painter.rect_filled(
        name_rect,
        3.0,
        Color32::from_rgba_unmultiplied(0x22, 0x44, 0x66, 0xDD),
    );
    painter.rect_stroke(
        name_rect,
        3.0,
        Stroke::new(1.0_f32, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
    painter.galley(
        egui::pos2(
            type_rect.center().x - type_gal.size().x * 0.5,
            type_rect.center().y - type_gal.size().y * 0.5,
        ),
        type_gal,
        Color32::WHITE,
    );
    painter.galley(
        egui::pos2(
            name_rect.min.x + pad_x,
            name_rect.center().y - name_gal.size().y * 0.5,
        ),
        name_gal,
        Color32::WHITE,
    );
    full.expand(2.0)
}

fn paint_typhoon_detail(painter: &egui::Painter, hit: &TyphoonHoverHit) {
    let time = format_typhoon_time(&hit.time_text);
    let header = if time.is_empty() {
        format!("{} {}", hit.strong, hit.name)
    } else {
        format!("{} {}  {}", hit.strong, hit.name, time)
    };
    let lat_s = format!("{:.1}", hit.latitude);
    let lon_s = format!("{:.1}", hit.longitude);
    let power = if hit.power > 0 {
        format!("{}级", hit.power)
    } else {
        "—".into()
    };
    let speed = if hit.speed_ms > 0.0 {
        format!("{:.0} m/s ({power})", hit.speed_ms)
    } else {
        format!("— ({power})")
    };
    let pressure = if hit.pressure_hpa > 0.0 {
        format!("{:.0} hPa", hit.pressure_hpa)
    } else {
        "—".into()
    };
    let mv = if hit.move_direction.is_empty() && hit.move_speed_kmh <= 0.0 {
        "—".into()
    } else {
        format!(
            "{} ({:.0} KM/H)",
            if hit.move_direction.is_empty() {
                "—"
            } else {
                hit.move_direction.as_str()
            },
            hit.move_speed_kmh
        )
    };
    let lines = [
        format!("编号: {}", hit.id),
        format!("中心位置: ({lat_s}°N, {lon_s}°E)"),
        format!("最大风速: {speed}"),
        format!("中心气压: {pressure}"),
        format!("移动速度: {mv}"),
    ];

    let font = egui::FontId::proportional(12.0);
    let header_gal = painter.layout_no_wrap(header, font.clone(), Color32::WHITE);
    let body: Vec<_> = lines
        .iter()
        .map(|l| painter.layout_no_wrap(l.clone(), font.clone(), Color32::WHITE))
        .collect();
    let body_w = body
        .iter()
        .map(|g| g.size().x)
        .fold(header_gal.size().x, f32::max)
        .max(200.0);
    let pad = 8.0_f32;
    let gap = 2.0_f32;
    let header_h = header_gal.size().y + pad;
    let body_h = body.iter().map(|g| g.size().y).sum::<f32>()
        + gap * (body.len().saturating_sub(1) as f32)
        + pad;
    let w = body_w + pad * 2.0;
    let origin = egui::pos2(hit.anchor.x + 12.0, hit.anchor.y + 14.0);
    let header_rect = Rect::from_min_size(origin, Vec2::new(w, header_h));
    let body_rect = Rect::from_min_size(
        egui::pos2(origin.x, origin.y + header_h + 2.0),
        Vec2::new(w, body_h),
    );
    painter.rect_filled(
        header_rect,
        3.0,
        Color32::from_rgba_unmultiplied(0x44, 0x88, 0xCC, 0xEE),
    );
    painter.rect_stroke(
        header_rect,
        3.0,
        Stroke::new(1.0_f32, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
    painter.rect_filled(
        body_rect,
        3.0,
        Color32::from_rgba_unmultiplied(0x11, 0x11, 0x22, 0xEE),
    );
    painter.rect_stroke(
        body_rect,
        3.0,
        Stroke::new(1.0_f32, Color32::from_rgb(0x66, 0x66, 0x88)),
        egui::StrokeKind::Outside,
    );
    painter.galley(
        egui::pos2(header_rect.min.x + pad, header_rect.center().y - header_gal.size().y * 0.5),
        header_gal,
        Color32::WHITE,
    );
    let mut y = body_rect.min.y + pad * 0.5;
    for g in body {
        let h = g.size().y;
        painter.galley(egui::pos2(body_rect.min.x + pad, y), g, Color32::WHITE);
        y += h + gap;
    }
}

fn normalize_typhoon_agency(a: &str) -> String {
    match a.trim() {
        "台湾" | "中国台湾" => "中国台湾".into(),
        "香港" | "中国香港" => "中国香港".into(),
        other => other.to_string(),
    }
}

fn format_typhoon_time(time_str: &str) -> String {
    // 2026-10-13 14:00:00 → 13日14时
    if let Some((d, h)) = regex_lite_ymd_hour(time_str) {
        return format!("{d}日{h}时");
    }
    time_str.to_string()
}

fn regex_lite_ymd_hour(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i + 10 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && (bytes[i + 4] == b'-' || bytes[i + 4] == b'/')
        {
            let day: u32 = std::str::from_utf8(&bytes[i + 8..i + 10])
                .ok()?
                .parse()
                .ok()?;
            let mut j = i + 10;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'T') {
                j += 1;
            }
            if j + 2 <= bytes.len()
                && bytes[j].is_ascii_digit()
                && bytes[j + 1].is_ascii_digit()
            {
                let hour: u32 = std::str::from_utf8(&bytes[j..j + 2]).ok()?.parse().ok()?;
                return Some((day, hour));
            }
        }
        i += 1;
    }
    None
}

fn paint_wind_circles(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    r7: Option<jian_core::TyphoonQuadrants>,
    r10: Option<jian_core::TyphoonQuadrants>,
    r12: Option<jian_core::TyphoonQuadrants>,
) {
    // 由外到内画：7 级最大 → 12 级最小，避免小圈被盖死
    if let Some(q) = r7 {
        paint_wind_quad(
            painter,
            view,
            lon,
            lat,
            q,
            Color32::from_rgba_unmultiplied(0x55, 0xFF, 0x55, 0x40),
            Color32::from_rgb(0x55, 0xFF, 0x55),
        );
    }
    if let Some(q) = r10 {
        paint_wind_quad(
            painter,
            view,
            lon,
            lat,
            q,
            Color32::from_rgba_unmultiplied(0xFF, 0xDD, 0x44, 0x48),
            Color32::from_rgb(0xFF, 0xDD, 0x44),
        );
    }
    if let Some(q) = r12 {
        paint_wind_quad(
            painter,
            view,
            lon,
            lat,
            q,
            Color32::from_rgba_unmultiplied(0xFF, 0x55, 0x55, 0x50),
            Color32::from_rgb(0xFF, 0x55, 0x55),
        );
    }
}

fn paint_wind_quad(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon: f64,
    lat: f64,
    q: jian_core::TyphoonQuadrants,
    fill: Color32,
    border: Color32,
) {
    let Some(center) = project_near(view, lon, lat) else {
        return;
    };
    // 连续方位采样：每象限用该象限半径；r=0 则落到中心（与 eewcn 跳过象限等价但闭合正确）
    let r_at = |az: f64| -> f64 {
        let a = az.rem_euclid(360.0);
        if a < 90.0 {
            q.ne
        } else if a < 180.0 {
            q.se
        } else if a < 270.0 {
            q.sw
        } else {
            q.nw
        }
    };
    let step = 4.0_f64;
    let mut ring: Vec<Pos2> = Vec::with_capacity(100);
    let mut az = 0.0_f64;
    while az < 360.0 - 1e-6 {
        let r_km = r_at(az);
        let p = if r_km > 0.0 {
            let (olon, olat) = crate::globe::move_on_globe(lon, lat, r_km, az);
            project_near(view, olon, olat)
        } else {
            Some(center)
        };
        if let Some(p) = p {
            ring.push(p);
        }
        az += step;
    }
    // 闭合：360°
    {
        let r_km = r_at(0.0);
        let p = if r_km > 0.0 {
            let (olon, olat) = crate::globe::move_on_globe(lon, lat, r_km, 0.0);
            project_near(view, olon, olat)
        } else {
            Some(center)
        };
        if let Some(p) = p {
            ring.push(p);
        }
    }
    if ring.len() < 3 {
        return;
    }
    // 中心扇形三角网填充（避免 PathShape 对凹/阶梯边界错误 tessellate）
    for w in ring.windows(2) {
        painter.add(egui::Shape::convex_polygon(
            vec![center, w[0], w[1]],
            fill,
            Stroke::NONE,
        ));
    }
    // 外轮廓描边（跳过落到中心的退化段）
    let mut stroke_pts: Vec<Pos2> = Vec::new();
    for p in &ring {
        let on_center = (p.x - center.x).abs() < 0.5 && (p.y - center.y).abs() < 0.5;
        if on_center {
            if stroke_pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    stroke_pts.clone(),
                    Stroke::new(1.8_f32, border),
                ));
            }
            stroke_pts.clear();
            continue;
        }
        stroke_pts.push(*p);
    }
    if stroke_pts.len() >= 2 {
        painter.add(egui::Shape::line(
            stroke_pts,
            Stroke::new(1.8_f32, border),
        ));
    }
}

fn paint_geodesic_segment(
    painter: &egui::Painter,
    view: &ViewCtx,
    lon0: f64,
    lat0: f64,
    lon1: f64,
    lat1: f64,
    stroke: Stroke,
) {
    // 分段插值；经度相对中心折绕，避免跨日界线拉直线
    let n = 8usize;
    let mut pts: Vec<Pos2> = Vec::with_capacity(n + 1);
    let lon0 = projection::wrap_lon_near(lon0, view.center_lon);
    let lon1 = projection::wrap_lon_near(lon1, view.center_lon);
    // 再折到相对 lon0 的最短弧
    let lon1 = lon0 + projection::normalize_lon(lon1 - lon0);
    for i in 0..=n {
        let t = i as f64 / n as f64;
        let lon = lon0 + (lon1 - lon0) * t;
        let lat = lat0 + (lat1 - lat0) * t;
        if let Some(p) = project_near(view, lon, lat) {
            if let Some(prev) = pts.last() {
                let dx = p.x - prev.x;
                let dy = p.y - prev.y;
                if dx * dx + dy * dy > 180.0_f32 * 180.0 {
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::line(pts.clone(), stroke));
                    }
                    pts.clear();
                }
            }
            pts.push(p);
        }
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, stroke));
    }
}
