//! 球体已填色贴图（备用）：等距圆柱 `globe_dark.png`。
//! 当前球体陆地改为 GQ 式矢量填色+地平线弓弧，本模块暂未接入 `MapViewport`。

use crate::projection::ViewCtx;
use egui::{Color32, ColorImage, Mesh, Pos2, TextureHandle, TextureOptions, Ui};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub struct GlobeTexture {
    path: Option<PathBuf>,
    tex: Option<TextureHandle>,
    tried: bool,
}

impl Default for GlobeTexture {
    fn default() -> Self {
        Self {
            path: None,
            tex: None,
            tried: false,
        }
    }
}

impl GlobeTexture {
    pub fn set_geodata_dir(&mut self, dir: Option<&Path>) {
        let next = dir.map(|d| d.join("globe_dark.png"));
        if next != self.path {
            self.path = next;
            self.tex = None;
            self.tried = false;
        }
    }

    fn ensure(&mut self, ctx: &egui::Context) {
        if self.tex.is_some() || self.tried {
            return;
        }
        self.tried = true;
        let Some(path) = self.path.clone() else {
            return;
        };
        if !path.is_file() {
            warn!(path = %path.display(), "globe texture missing");
            return;
        }
        match load_png(&path) {
            Some(img) => {
                info!(
                    path = %path.display(),
                    w = img.size[0],
                    h = img.size[1],
                    "loaded globe land texture"
                );
                // 文件名带像素尺寸，避免 egui 纹理缓存命中旧低清图
                let key = format!("eewview/globe_dark_{}x{}", img.size[0], img.size[1]);
                self.tex = Some(ctx.load_texture(
                    key,
                    img,
                    TextureOptions {
                        magnification: egui::TextureFilter::Linear,
                        minification: egui::TextureFilter::Linear,
                        wrap_mode: egui::TextureWrapMode::ClampToEdge,
                        mipmap_mode: None,
                    },
                ));
            }
            None => warn!(path = %path.display(), "globe texture decode failed"),
        }
    }

    /// 在球体可见半球上绘制已填色陆地贴图。
    pub fn paint(&mut self, ui: &mut Ui, view: &ViewCtx) {
        if !view.mode.is_globe() {
            return;
        }
        self.ensure(ui.ctx());
        let Some(tex) = self.tex.as_ref() else {
            return;
        };
        let Some(mesh) = build_globe_mesh(view, tex) else {
            return;
        };
        ui.painter_at(view.rect).add(egui::Shape::mesh(mesh));
    }
}

fn load_png(path: &Path) -> Option<ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// 用未折绕经度算 UV，保证相邻格 UV 连续；采样前再归一到 [0,1]。
fn lonlat_uv_raw(lon: f64, lat: f64) -> Pos2 {
    let u = ((lon + 180.0) / 360.0) as f32;
    let v = ((90.0 - lat) / 180.0).clamp(0.0, 1.0) as f32;
    Pos2::new(u, v)
}

fn build_globe_mesh(view: &ViewCtx, tex: &TextureHandle) -> Option<Mesh> {
    // 按视口覆盖范围 + 屏幕格距建网：放大后若仍铺 ±100° 且 max_jump≤140，
    // 相邻格边长会超过阈值，三角形被整批丢掉，只剩中间一块「窗口」。
    let one_deg = view.one_deg_px().unwrap_or(8.0).max(1e-3);
    let margin = 1.4_f64;
    let half_w_deg = (view.rect.width() as f64 * 0.5 + 48.0) / one_deg * margin;
    let half_h_deg = (view.rect.height() as f64 * 0.5 + 48.0) / one_deg * margin;
    let lon_span = (half_w_deg * 2.0).clamp(4.0, 200.0);
    let lat_half = half_h_deg.clamp(2.0, 90.0);
    let lat0 = (view.center_lat - lat_half).max(-90.0);
    let lat1 = (view.center_lat + lat_half).min(90.0);
    let lat_span = (lat1 - lat0).max(2.0);
    let lon0 = view.center_lon - lon_span * 0.5;
    let lon1 = view.center_lon + lon_span * 0.5;

    // 目标格边约 28px，随视口加密，上限避免过密
    const TARGET_CELL_PX: f64 = 28.0;
    let n_lon = ((lon_span * one_deg / TARGET_CELL_PX).ceil() as usize).clamp(32, 384);
    let n_lat = ((lat_span * one_deg / TARGET_CELL_PX).ceil() as usize).clamp(24, 288);

    let mut mesh = Mesh::with_texture(tex.id());
    mesh.reserve_vertices((n_lat + 1) * (n_lon + 1));
    mesh.reserve_triangles(n_lat * n_lon * 2);

    // 略扩出视口，让边缘格仍有四点，交给 painter clip
    let keep = view.rect.expand(96.0);
    let mut grid: Vec<Option<(Pos2, Pos2)>> = Vec::with_capacity((n_lat + 1) * (n_lon + 1));
    for i in 0..=n_lat {
        let t = i as f64 / n_lat as f64;
        let lat = lat0 + lat_span * t;
        for j in 0..=n_lon {
            let s = j as f64 / n_lon as f64;
            let lon = lon0 + (lon1 - lon0) * s; // 连续未折绕
            let lon_n = crate::projection::normalize_lon(lon);
            match view.project(lon_n, lat) {
                Some(sp) if keep.contains(sp) => {
                    grid.push(Some((sp, lonlat_uv_raw(lon, lat))));
                }
                _ => grid.push(None),
            }
        }
    }

    let stride = n_lon + 1;
    // 跳变阈值跟预期格距走，放大后允许更大边长（只挡穿球/接缝弦）
    let expect_px = (lon_span / n_lon as f64 * one_deg)
        .max(lat_span / n_lat as f64 * one_deg)
        .max(TARGET_CELL_PX);
    let max_jump = (expect_px * 3.5).clamp(56.0, 900.0) as f32;

    for i in 0..n_lat {
        for j in 0..n_lon {
            let i00 = i * stride + j;
            let i10 = i00 + 1;
            let i01 = i00 + stride;
            let i11 = i01 + 1;
            let (Some((p00, uv00)), Some((p10, uv10)), Some((p01, uv01)), Some((p11, uv11))) = (
                grid[i00], grid[i10], grid[i01], grid[i11],
            ) else {
                continue;
            };
            if (p00 - p10).length() > max_jump
                || (p00 - p01).length() > max_jump
                || (p10 - p11).length() > max_jump
                || (p01 - p11).length() > max_jump
            {
                continue;
            }
            // 日界线：UV.u 跨度过大说明跨接缝，整格丢弃（避免竖条/糊条）
            if uv_span_crosses_seam(&[uv00, uv10, uv01, uv11]) {
                continue;
            }
            let (uv00, uv10, uv01, uv11) = normalize_uv_quad(uv00, uv10, uv01, uv11);
            push_textured_tri(&mut mesh, p00, uv00, p10, uv10, p11, uv11);
            push_textured_tri(&mut mesh, p00, uv00, p11, uv11, p01, uv01);
        }
    }

    if mesh.indices.is_empty() {
        None
    } else {
        Some(mesh)
    }
}

fn uv_span_crosses_seam(uvs: &[Pos2]) -> bool {
    let mut minu = f32::INFINITY;
    let mut maxu = f32::NEG_INFINITY;
    for uv in uvs {
        minu = minu.min(uv.x);
        maxu = maxu.max(uv.x);
    }
    maxu - minu > 0.5
}

/// 将四角 UV 平移到同一纹理周期内，再夹到 [0,1]。
fn normalize_uv_quad(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> (Pos2, Pos2, Pos2, Pos2) {
    let base = a.x.min(b.x).min(c.x).min(d.x).floor();
    let norm = |p: Pos2| Pos2::new((p.x - base).clamp(0.0, 1.0), p.y.clamp(0.0, 1.0));
    (norm(a), norm(b), norm(c), norm(d))
}

fn push_textured_tri(
    mesh: &mut Mesh,
    a: Pos2,
    uva: Pos2,
    b: Pos2,
    uvb: Pos2,
    c: Pos2,
    uvc: Pos2,
) {
    let base = mesh.vertices.len() as u32;
    mesh.vertices.push(egui::epaint::Vertex {
        pos: a,
        uv: uva,
        color: Color32::WHITE,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: b,
        uv: uvb,
        color: Color32::WHITE,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: c,
        uv: uvc,
        color: Color32::WHITE,
    });
    mesh.add_triangle(base, base + 1, base + 2);
}
