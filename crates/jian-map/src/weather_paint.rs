//! 晨昏线（平面 / 球体均可）。

use crate::projection::ViewCtx;
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Pos2, Shape, Stroke, Ui};
use jian_core::{solar_altitude_deg, sun_geometry};
use std::collections::{HashMap, HashSet};

/// 夜侧半透明灰、暮光渐变，以及沿高度角零线的橙金辉光。
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

    let lines = terminator_polylines(&pos, &alt, nx, ny);
    for pts in lines {
        if pts.len() < 2 {
            continue;
        }
        painter.line(
            pts.clone(),
            Stroke::new(12.0_f32, Color32::from_rgba_unmultiplied(255, 148, 32, 32)),
        );
        painter.line(
            pts.clone(),
            Stroke::new(6.5_f32, Color32::from_rgba_unmultiplied(255, 168, 48, 78)),
        );
        painter.line(
            pts.clone(),
            Stroke::new(2.6_f32, Color32::from_rgba_unmultiplied(255, 184, 60, 220)),
        );
        painter.line(
            pts,
            Stroke::new(1.15_f32, Color32::from_rgba_unmultiplied(255, 232, 176, 250)),
        );
    }
}

/// 高度角 ≤ 0 的一侧：约 6.5° 暮光内由透明过渡到半透明灰。
fn night_tint(alt_deg: f32) -> Color32 {
    if alt_deg >= 0.0 {
        return Color32::TRANSPARENT;
    }
    let t = ((-alt_deg) / 6.5).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    let a = (t * 148.0) as u8;
    if a == 0 {
        return Color32::TRANSPARENT;
    }
    Color32::from_rgba_unmultiplied(42, 46, 54, a)
}

fn terminator_polylines(pos: &[Pos2], alt: &[f32], nx: i32, ny: i32) -> Vec<Vec<Pos2>> {
    let mut segs: Vec<(Pos2, Pos2)> = Vec::new();
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
            let corners = [
                (pos[ids[0]], alt[ids[0]]),
                (pos[ids[1]], alt[ids[1]]),
                (pos[ids[2]], alt[ids[2]]),
                (pos[ids[3]], alt[ids[3]]),
            ];
            segs.extend(cell_segments(corners));
        }
    }
    stitch_segments(segs)
}

/// 格子四角：0 左上、1 右上、2 右下、3 左下。白天为高度角 ≥ 0。
fn cell_segments(c: [(Pos2, f32); 4]) -> Vec<(Pos2, Pos2)> {
    let mut mask = 0u8;
    for (i, (_, a)) in c.iter().enumerate() {
        if *a >= 0.0 {
            mask |= 1 << i;
        }
    }
    if mask == 0 || mask == 15 {
        return Vec::new();
    }
    let edge = |a: usize, b: usize| lerp_zero(c[a].0, c[a].1, c[b].0, c[b].1);
    let top = edge(0, 1);
    let right = edge(1, 2);
    let bottom = edge(2, 3);
    let left = edge(3, 0);
    let center = (c[0].1 + c[1].1 + c[2].1 + c[3].1) * 0.25;
    match mask {
        1 | 14 => vec![(left, top)],
        2 | 13 => vec![(top, right)],
        3 | 12 => vec![(left, right)],
        4 | 11 => vec![(right, bottom)],
        6 | 9 => vec![(top, bottom)],
        7 | 8 => vec![(bottom, left)],
        5 => {
            if center >= 0.0 {
                vec![(left, bottom), (top, right)]
            } else {
                vec![(left, top), (right, bottom)]
            }
        }
        10 => {
            if center >= 0.0 {
                vec![(left, top), (right, bottom)]
            } else {
                vec![(left, bottom), (top, right)]
            }
        }
        _ => Vec::new(),
    }
}

fn lerp_zero(p0: Pos2, a0: f32, p1: Pos2, a1: f32) -> Pos2 {
    let den = a0 - a1;
    let t = if den.abs() < 1e-6 {
        0.5
    } else {
        (a0 / den).clamp(0.0, 1.0)
    };
    Pos2::new(p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t)
}

fn stitch_segments(segs: Vec<(Pos2, Pos2)>) -> Vec<Vec<Pos2>> {
    fn key(p: Pos2) -> (i32, i32) {
        ((p.x * 4.0).round() as i32, (p.y * 4.0).round() as i32)
    }
    fn canon(a: (i32, i32), b: (i32, i32)) -> ((i32, i32), (i32, i32)) {
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    }

    let mut adj: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    let mut at: HashMap<(i32, i32), Pos2> = HashMap::new();
    for (a, b) in segs {
        let ka = key(a);
        let kb = key(b);
        if ka == kb {
            continue;
        }
        adj.entry(ka).or_default().push(kb);
        adj.entry(kb).or_default().push(ka);
        at.insert(ka, a);
        at.insert(kb, b);
    }

    let mut used: HashSet<((i32, i32), (i32, i32))> = HashSet::new();
    let mut lines = Vec::new();
    let starts: Vec<(i32, i32)> = adj.keys().copied().collect();
    for start in starts {
        let Some(neigh) = adj.get(&start).cloned() else {
            continue;
        };
        for n0 in neigh {
            if !used.insert(canon(start, n0)) {
                continue;
            }
            let mut chain = vec![start, n0];
            let mut prev = start;
            let mut cur = n0;
            loop {
                let Some(nexts) = adj.get(&cur) else { break };
                let nxt = nexts.iter().copied().find(|nx| {
                    *nx != prev && !used.contains(&canon(cur, *nx))
                });
                let Some(nx) = nxt else { break };
                used.insert(canon(cur, nx));
                chain.push(nx);
                prev = cur;
                cur = nx;
                if chain.len() > 200_000 {
                    break;
                }
            }
            if chain.len() >= 2 {
                prev = chain[1];
                cur = chain[0];
                loop {
                    let Some(nexts) = adj.get(&cur) else { break };
                    let nxt = nexts.iter().copied().find(|nx| {
                        *nx != prev && !used.contains(&canon(cur, *nx))
                    });
                    let Some(nx) = nxt else { break };
                    used.insert(canon(cur, nx));
                    chain.insert(0, nx);
                    prev = cur;
                    cur = nx;
                    if chain.len() > 200_000 {
                        break;
                    }
                }
            }
            let pts: Vec<Pos2> = chain.iter().filter_map(|k| at.get(k).copied()).collect();
            if pts.len() >= 2 {
                lines.push(pts);
            }
        }
    }
    lines
}
