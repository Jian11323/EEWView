//! 晨昏线（平面 / 球体均可）：仅夜侧明暗渐变，无硬边描线。

use crate::projection::ViewCtx;
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Pos2, Shape, Ui};
use jian_core::{solar_altitude_deg, sun_geometry};

/// 按太阳高度角铺夜侧半透明罩：民用暮光内柔和过渡，无橙金描边。
pub fn paint_terminator(ui: &mut Ui, view: &ViewCtx, now_ms: i64) {
    let rect = view.rect;
    if rect.width() < 8.0 || rect.height() < 8.0 {
        return;
    }
    let (dec, ha0) = sun_geometry(now_ms);
    let step = 7.0_f32;
    let nx = ((rect.width() / step).ceil() as i32).clamp(2, 420) + 1;
    let ny = ((rect.height() / step).ceil() as i32).clamp(2, 280) + 1;
    let n = (nx * ny) as usize;
    let mut pos = vec![Pos2::ZERO; n];
    let mut alt = vec![f32::NAN; n];

    let horizon = view.horizon_radius();
    let center = rect.center();
    for j in 0..ny {
        for i in 0..nx {
            let p = Pos2::new(
                rect.left() + i as f32 * rect.width() / (nx - 1) as f32,
                rect.top() + j as f32 * rect.height() / (ny - 1) as f32,
            );
            let idx = (j * nx + i) as usize;
            pos[idx] = p;
            if let Some(r) = horizon {
                if p.distance(center) > r {
                    continue;
                }
            }
            let (lon, lat) = view.screen_to_lonlat(p);
            if view.mode.is_globe() && !view.is_above_horizon(lon, lat) {
                continue;
            }
            alt[idx] = solar_altitude_deg(lon, lat, dec, ha0) as f32;
        }
    }

    let painter = ui.painter_at(rect);
    let mut mesh = Mesh::default();
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let ids = [
                (j * nx + i) as usize,
                (j * nx + i + 1) as usize,
                ((j + 1) * nx + i + 1) as usize,
                ((j + 1) * nx + i) as usize,
            ];
            if ids.iter().any(|&k| !alt[k].is_finite()) {
                continue;
            }
            let colors = [
                night_tint(alt[ids[0]]),
                night_tint(alt[ids[1]]),
                night_tint(alt[ids[2]]),
                night_tint(alt[ids[3]]),
            ];
            if colors.iter().all(|c| c.a() == 0) {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for k in 0..4 {
                mesh.vertices.push(Vertex {
                    pos: pos[ids[k]],
                    uv: Pos2::ZERO,
                    color: colors[k],
                });
            }
            mesh.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    if !mesh.vertices.is_empty() {
        painter.add(Shape::mesh(mesh));
    }
}

/// 太阳高度角：约 +1.5° 起微暗，-12° 达夜侧满罩；smoothstep，无硬边。
fn night_tint(alt_deg: f32) -> Color32 {
    // 民用暮光约 0°～-6°，再延伸到航海暮光 -12°，日侧略提前起晕避免零线硬切。
    const ALT_START: f32 = 1.5;
    const ALT_FULL: f32 = -12.0;
    if alt_deg >= ALT_START {
        return Color32::TRANSPARENT;
    }
    let span = ALT_START - ALT_FULL;
    let t = ((ALT_START - alt_deg) / span).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let a = (t * 158.0) as u8;
    if a == 0 {
        return Color32::TRANSPARENT;
    }
    Color32::from_rgba_unmultiplied(36, 40, 50, a)
}
