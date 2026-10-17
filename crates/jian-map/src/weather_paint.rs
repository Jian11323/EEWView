//! 晨昏线与风场箭头（平面 / 球体均可）。

use crate::projection::ViewCtx;
use egui::{Color32, Pos2, Stroke, Ui};
use jian_core::{is_night, WindGrid};

pub fn paint_terminator(ui: &mut Ui, view: &ViewCtx, now_ms: i64) {
    let rect = view.rect;
    let painter = ui.painter_at(rect);
    let step = if view.mode.is_globe() { 10.0_f32 } else { 8.0_f32 };
    let shade = Color32::from_rgba_unmultiplied(4, 10, 28, 110);
    let mut y = rect.top();
    while y < rect.bottom() {
        let mut x = rect.left();
        let mut run_start: Option<f32> = None;
        while x <= rect.right() + step {
            let px = x.min(rect.right());
            let (lon, lat) = view.screen_to_lonlat(Pos2::new(px, y));
            let on_globe = !view.mode.is_globe() || view.is_above_horizon(lon, lat);
            let night = on_globe && is_night(lon, lat, now_ms);
            if night && run_start.is_none() {
                run_start = Some(x);
            }
            if !night {
                if let Some(xs) = run_start.take() {
                    let r = egui::Rect::from_min_max(
                        Pos2::new(xs, y),
                        Pos2::new(x, (y + step).min(rect.bottom())),
                    );
                    painter.rect_filled(r, 0.0, shade);
                }
            }
            x += step;
        }
        if let Some(xs) = run_start {
            let r = egui::Rect::from_min_max(
                Pos2::new(xs, y),
                Pos2::new(rect.right(), (y + step).min(rect.bottom())),
            );
            painter.rect_filled(r, 0.0, shade);
        }
        y += step;
    }
}

pub fn paint_wind(ui: &mut Ui, view: &ViewCtx, grid: &WindGrid) {
    let rect = view.rect;
    let step = match view.zoom {
        z if z < 3.0 => 48.0_f32,
        z if z < 5.0 => 36.0_f32,
        _ => 28.0_f32,
    };
    let painter = ui.painter_at(rect);
    let mut y = rect.top() + step * 0.5;
    while y < rect.bottom() {
        let mut x = rect.left() + step * 0.5;
        while x < rect.right() {
            let (lon, lat) = view.screen_to_lonlat(Pos2::new(x, y));
            if view.mode.is_globe() && !view.is_above_horizon(lon, lat) {
                x += step;
                continue;
            }
            if let Some((u, v)) = grid.sample(lon, lat) {
                let spd = (u * u + v * v).sqrt();
                if spd > 1.5 {
                    let len = (8.0 + spd.min(25.0) * 0.6).min(22.0);
                    let ang = (-v).atan2(u);
                    let dx = ang.cos() * len;
                    let dy = ang.sin() * len;
                    let a = Pos2::new(x, y);
                    let b = Pos2::new(x + dx, y + dy);
                    let col = Color32::from_rgba_unmultiplied(180, 220, 255, 180);
                    painter.line_segment([a, b], Stroke::new(1.2_f32, col));
                }
            }
            x += step;
        }
        y += step;
    }
}
