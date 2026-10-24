//! 雷达 PNG / 降水多边形地图叠加。
//!
//! 雷达：平面单四边形；球体按经纬网格加密贴面并每帧重投影，随球转动。

use crate::projection::ViewCtx;
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, ColorImage, Context, Pos2, Shape, TextureHandle, TextureOptions, Ui};
use starquake_core::{PrecipFrame, RadarFrame};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
pub struct WeatherOverlay {
    /// 各路雷达纹理缓存
    radar_tex: HashMap<&'static str, (u64, TextureHandle, Arc<Vec<u8>>)>,
}

impl WeatherOverlay {
    pub fn paint_radars(
        &mut self,
        ui: &mut Ui,
        view: &ViewCtx,
        frames: &[&RadarFrame],
        opacity: f32,
    ) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.001 {
            return;
        }
        let live: Vec<&'static str> = frames.iter().map(|f| f.kind.as_str()).collect();
        self.radar_tex.retain(|k, _| live.iter().any(|l| l == k));

        for frame in frames {
            self.paint_one_radar(ui, view, frame, opacity);
        }
    }

    fn paint_one_radar(
        &mut self,
        ui: &mut Ui,
        view: &ViewCtx,
        frame: &RadarFrame,
        opacity: f32,
    ) {
        let key = radar_key(frame);
        let kind = frame.kind.as_str();
        let need_load = match self.radar_tex.get(kind) {
            Some((k, _, _)) => *k != key,
            None => true,
        };
        if need_load {
            let Some(tex) = load_radar_tex(ui.ctx(), frame) else {
                return;
            };
            self.radar_tex
                .insert(kind, (key, tex, Arc::clone(&frame.png)));
        }
        let Some((_, tex, _)) = self.radar_tex.get(kind) else {
            return;
        };
        let tex_id = tex.id();

        let globe = view.mode.is_globe();
        // 球体加密贴面；平面单格即可
        let (nx, ny) = if globe { (28usize, 18usize) } else { (1, 1) };

        let alpha = (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        let tint = Color32::from_rgba_unmultiplied(255, 255, 255, alpha);
        let mut mesh = Mesh::with_texture(tex_id);

        let sw_lon = frame.sw_lon;
        let sw_lat = frame.sw_lat;
        let ne_lon = frame.ne_lon;
        let ne_lat = frame.ne_lat;

        for iy in 0..ny {
            for ix in 0..nx {
                // u: 西→东；v: 北→南（贴图 y=0 为北）
                let u0 = ix as f64 / nx as f64;
                let u1 = (ix + 1) as f64 / nx as f64;
                let v0 = iy as f64 / ny as f64;
                let v1 = (iy + 1) as f64 / ny as f64;
                let corners = [
                    (lerp(sw_lon, ne_lon, u0), lerp(ne_lat, sw_lat, v0), u0 as f32, v0 as f32),
                    (lerp(sw_lon, ne_lon, u1), lerp(ne_lat, sw_lat, v0), u1 as f32, v0 as f32),
                    (lerp(sw_lon, ne_lon, u1), lerp(ne_lat, sw_lat, v1), u1 as f32, v1 as f32),
                    (lerp(sw_lon, ne_lon, u0), lerp(ne_lat, sw_lat, v1), u0 as f32, v1 as f32),
                ];
                // NW, NE, SE, SW in screen → UV already set
                let mut pts = [None; 4];
                for (i, &(lon, lat, _, _)) in corners.iter().enumerate() {
                    pts[i] = if globe {
                        view.project_fill(lon, lat)
                    } else {
                        view.project(lon, lat)
                    };
                }
                // 至少 3 角可见才画；球体背面格跳过
                let visible = pts.iter().filter(|p| p.is_some()).count();
                if visible < 3 {
                    continue;
                }
                // 四角齐全：两三角；缺一角：用三可见点扇形（退化）
                if let (Some(a), Some(b), Some(c), Some(d)) = (pts[0], pts[1], pts[2], pts[3]) {
                    let base = mesh.vertices.len() as u32;
                    for (i, p) in [a, b, c, d].into_iter().enumerate() {
                        mesh.vertices.push(Vertex {
                            pos: p,
                            uv: Pos2::new(corners[i].2, corners[i].3),
                            color: tint,
                        });
                    }
                    mesh.indices
                        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }

        if !mesh.vertices.is_empty() {
            ui.painter_at(view.rect).add(Shape::mesh(mesh));
        }
    }

    pub fn paint_precip(
        ui: &mut Ui,
        view: &ViewCtx,
        frame: Option<&PrecipFrame>,
        opacity: f32,
    ) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.001 {
            return;
        }
        let Some(frame) = frame else {
            return;
        };
        if frame.polys.is_empty() {
            return;
        }
        let painter = ui.painter_at(view.rect);
        let mut mesh = Mesh::default();
        for poly in &frame.polys {
            let a = ((f32::from(poly.fill[3]) * opacity).round()).clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(
                poly.fill[0],
                poly.fill[1],
                poly.fill[2],
                a,
            );
            let mut pts = Vec::with_capacity(poly.ring.len());
            for p in &poly.ring {
                if let Some(sp) = view.project(p[0], p[1]) {
                    pts.push(sp);
                } else {
                    flush_poly(&mut mesh, &mut pts, color);
                }
            }
            flush_poly(&mut mesh, &mut pts, color);
        }
        if !mesh.vertices.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn flush_poly(mesh: &mut Mesh, pts: &mut Vec<Pos2>, color: Color32) {
    if pts.len() < 3 {
        pts.clear();
        return;
    }
    let origin = pts[0];
    for w in pts[1..].windows(2) {
        let i0 = mesh.vertices.len() as u32;
        mesh.colored_vertex(origin, color);
        mesh.colored_vertex(w[0], color);
        mesh.colored_vertex(w[1], color);
        mesh.add_triangle(i0, i0 + 1, i0 + 2);
    }
    pts.clear();
}

fn radar_key(frame: &RadarFrame) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    frame.kind.as_str().hash(&mut h);
    frame.time.hash(&mut h);
    frame.png.len().hash(&mut h);
    if let Some(b) = frame.png.first() {
        b.hash(&mut h);
    }
    if let Some(b) = frame.png.last() {
        b.hash(&mut h);
    }
    h.finish()
}

fn load_radar_tex(ctx: &Context, frame: &RadarFrame) -> Option<TextureHandle> {
    let img = image::load_from_memory(frame.png.as_ref()).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    // 雷达是离散色阶拼图：LINEAR 会把相邻色阶混成糊斑；放大用 Nearest 保色块
    Some(ctx.load_texture(
        format!("starquake/radar/{}x{}", frame.kind.as_str(), size[0]),
        color,
        TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            wrap_mode: egui::TextureWrapMode::ClampToEdge,
            mipmap_mode: Some(egui::TextureFilter::Linear),
        },
    ))
}
