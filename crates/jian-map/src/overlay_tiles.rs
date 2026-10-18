//! 气象叠层：XYZ 瓦片缓存与本站全球雷达单幅，与底图缓存分离。

use crate::projection::{self, ViewCtx, TILE_SIZE};
use egui::epaint::Vertex;
use egui::{
    Color32, ColorImage, Context, Mesh, Pos2, Rect, TextureHandle, TextureOptions, Ui,
};
use jian_core::CloudSheet;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const MAX_CACHED: usize = 96;
const MAX_IN_FLIGHT: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TileId {
    z: u8,
    x: u32,
    y: u32,
}

enum TileState {
    Pending,
    Ready(TextureHandle),
    Failed,
}

struct DecodedTile {
    id: TileId,
    image: Option<ColorImage>,
}

pub struct OverlayTileCache {
    /// 完整前缀，不含 z/x/y，如 `https://host/v2/radar/abc/256`
    prefix: String,
    /// 瓦片后缀，如 `/2/1_0.png`
    suffix: String,
    tint: Color32,
    entries: HashMap<TileId, TileState>,
    lru: VecDeque<TileId>,
    tx: Sender<DecodedTile>,
    rx: Receiver<DecodedTile>,
    in_flight: usize,
    want: VecDeque<TileId>,
    max_z: u8,
}

impl OverlayTileCache {
    pub fn new(suffix: impl Into<String>, tint: Color32) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            prefix: String::new(),
            suffix: suffix.into(),
            tint,
            entries: HashMap::new(),
            lru: VecDeque::new(),
            tx,
            rx,
            in_flight: 0,
            want: VecDeque::new(),
            max_z: 7,
        }
    }

    pub fn set_max_z(&mut self, z: u8) {
        self.max_z = z.clamp(1, 18);
    }

    pub fn set_prefix(&mut self, prefix: &str) {
        let p = prefix.trim_end_matches('/').to_string();
        if p == self.prefix {
            return;
        }
        self.prefix = p;
        self.entries.clear();
        self.lru.clear();
        self.want.clear();
        self.in_flight = 0;
    }

    fn url(&self, id: TileId) -> String {
        format!(
            "{}/{}/{}/{}{}",
            self.prefix, id.z, id.x, id.y, self.suffix
        )
    }

    fn enqueue(&mut self, id: TileId) {
        if self.entries.contains_key(&id) || self.want.iter().any(|k| *k == id) {
            return;
        }
        self.want.push_back(id);
        self.entries.insert(id, TileState::Pending);
    }

    fn spawn_jobs(&mut self) {
        while self.in_flight < MAX_IN_FLIGHT {
            let Some(id) = self.want.pop_front() else {
                break;
            };
            if !matches!(self.entries.get(&id), Some(TileState::Pending)) {
                continue;
            }
            let url = self.url(id);
            let tx = self.tx.clone();
            self.in_flight += 1;
            let _ = thread::Builder::new().name("wx-tile".into()).spawn(move || {
                let image = fetch_tile(&url);
                let _ = tx.send(DecodedTile { id, image });
            });
        }
    }

    fn poll(&mut self, ctx: &Context) {
        let mut got = false;
        while let Ok(decoded) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            got = true;
            match decoded.image {
                Some(img) => {
                    let tex = ctx.load_texture(
                        format!(
                            "wx-{}-{}-{}-{}",
                            self.prefix.len(),
                            decoded.id.z,
                            decoded.id.x,
                            decoded.id.y
                        ),
                        img,
                        TextureOptions::LINEAR,
                    );
                    self.entries.insert(decoded.id, TileState::Ready(tex));
                    if let Some(pos) = self.lru.iter().position(|k| *k == decoded.id) {
                        self.lru.remove(pos);
                    }
                    self.lru.push_back(decoded.id);
                    while self.lru.len() > MAX_CACHED {
                        if let Some(old) = self.lru.pop_front() {
                            self.entries.remove(&old);
                        }
                    }
                }
                None => {
                    self.entries.insert(decoded.id, TileState::Failed);
                }
            }
        }
        self.spawn_jobs();
        if got || self.in_flight > 0 || !self.want.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub fn paint(&mut self, ui: &mut Ui, view: &ViewCtx) {
        if self.prefix.is_empty() {
            return;
        }
        self.poll(ui.ctx());
        let zoom = view.zoom;
        let zoom_tiles = zoom.clamp(0.0, self.max_z as f64);
        let z = zoom_tiles.floor().clamp(0.0, self.max_z as f64) as u8;
        let zf = z as f64;
        let scale = (2f64.powf(zoom - zf)) as f32;
        let tile_px = (TILE_SIZE as f32) * scale;
        let (_, x0, y0, x1, y1) = projection::visible_tiles(
            view.center_lon,
            view.center_lat,
            zoom_tiles,
            view.rect,
            if view.mode.is_globe() { 1 } else { 0 },
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.enqueue(TileId {
                    z,
                    x: projection::wrap_tile_x(x, z),
                    y: y as u32,
                });
            }
        }
        self.spawn_jobs();
        let painter = ui.painter_at(view.rect);
        let (cx, cy) = projection::lonlat_to_world(view.center_lon, view.center_lat, zf);
        let origin = view.rect.center();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let id = TileId {
                    z,
                    x: projection::wrap_tile_x(x, z),
                    y: y as u32,
                };
                let Some(TileState::Ready(tex)) = self.entries.get(&id) else {
                    continue;
                };
                let tid = tex.id();
                if view.mode.is_globe() {
                    // 整瓦四角在低缩放时会跨过地平线，整块被丢掉；拆成小格只画朝向相机的部分
                    let (ox, oy) = projection::tile_world_origin(x, y, z);
                    let (ox1, oy1) = projection::tile_world_origin(x + 1, y + 1, z);
                    const N: i32 = 4;
                    for gy in 0..N {
                        for gx in 0..N {
                            let fx0 = gx as f64 / N as f64;
                            let fx1 = (gx + 1) as f64 / N as f64;
                            let fy0 = gy as f64 / N as f64;
                            let fy1 = (gy + 1) as f64 / N as f64;
                            let wx0 = ox + (ox1 - ox) * fx0;
                            let wx1 = ox + (ox1 - ox) * fx1;
                            let wy0 = oy + (oy1 - oy) * fy0;
                            let wy1 = oy + (oy1 - oy) * fy1;
                            paint_projected_quad(
                                &painter,
                                view,
                                tid,
                                self.tint,
                                [
                                    projection::world_to_lonlat(wx0, wy0, zf),
                                    projection::world_to_lonlat(wx1, wy0, zf),
                                    projection::world_to_lonlat(wx1, wy1, zf),
                                    projection::world_to_lonlat(wx0, wy1, zf),
                                ],
                                [
                                    [fx0 as f32, fy0 as f32],
                                    [fx1 as f32, fy0 as f32],
                                    [fx1 as f32, fy1 as f32],
                                    [fx0 as f32, fy1 as f32],
                                ],
                                false,
                            );
                        }
                    }
                } else {
                    let (twx, twy) = projection::tile_world_origin(x, y, z);
                    let screen_min = Pos2::new(
                        origin.x + ((twx - cx) as f32) * scale,
                        origin.y + ((twy - cy) as f32) * scale,
                    );
                    let tile_rect = Rect::from_min_size(screen_min, egui::vec2(tile_px, tile_px));
                    painter.image(
                        tid,
                        tile_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        self.tint,
                    );
                }
            }
        }
    }
}

fn paint_projected_quad(
    painter: &egui::Painter,
    view: &ViewCtx,
    tid: egui::TextureId,
    tint: Color32,
    corners_ll: [(f64, f64); 4],
    uvs: [[f32; 2]; 4],
    // 球体须四角都在地平线内；缺角插值会拉成灰矩形贴纸。
    require_all: bool,
) {
    let mut pts = [None; 4];
    let mut n_ok = 0usize;
    for (i, (lon, lat)) in corners_ll.iter().enumerate() {
        pts[i] = view.project(*lon, *lat);
        if pts[i].is_some() {
            n_ok += 1;
        }
    }
    if require_all {
        if n_ok < 4 {
            return;
        }
    } else if n_ok < 3 {
        return;
    }
    let mut mesh = Mesh::with_texture(tid);
    for i in 0..4 {
        let Some(pos) = pts[i] else {
            // 仅瓦片路径允许缺角：用邻点中点顶上，仍可能略糊，但比整块消失好
            let a = pts[(i + 3) % 4];
            let b = pts[(i + 1) % 4];
            let pos = match (a, b) {
                (Some(pa), Some(pb)) => Pos2::new((pa.x + pb.x) * 0.5, (pa.y + pb.y) * 0.5),
                (Some(pa), None) | (None, Some(pa)) => pa,
                _ => return,
            };
            mesh.vertices.push(Vertex {
                pos,
                uv: egui::pos2(uvs[i][0], uvs[i][1]),
                color: tint,
            });
            continue;
        };
        mesh.vertices.push(Vertex {
            pos,
            uv: egui::pos2(uvs[i][0], uvs[i][1]),
            color: tint,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// 球体贴面：四角投影后若弦过长/绕序不一致/中心落背面，则视为穿球三角，丢弃。
fn globe_quad_screen_ok(pts: [Pos2; 4], max_edge: f32) -> bool {
    for i in 0..4 {
        if pts[i].distance(pts[(i + 1) % 4]) > max_edge {
            return false;
        }
    }
    if pts[0].distance(pts[2]) > max_edge * 1.45 || pts[1].distance(pts[3]) > max_edge * 1.45 {
        return false;
    }
    let cross = |a: Pos2, b: Pos2, c: Pos2| {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    };
    let c0 = cross(pts[0], pts[1], pts[2]);
    let c1 = cross(pts[0], pts[2], pts[3]);
    // 蝴蝶结或退化
    if c0 * c1 <= 0.0 {
        return false;
    }
    // 单三角面积过大（相对边长上限的正方形）
    let area = (c0.abs() + c1.abs()) * 0.5;
    if area > max_edge * max_edge * 1.2 {
        return false;
    }
    true
}

fn paint_globe_quad(
    painter: &egui::Painter,
    view: &ViewCtx,
    tid: egui::TextureId,
    tint: Color32,
    corners_ll: [(f64, f64); 4],
    uvs: [[f32; 2]; 4],
    max_edge: f32,
) -> bool {
    let mut pts = [Pos2::ZERO; 4];
    for i in 0..4 {
        let Some(p) = view.project(corners_ll[i].0, corners_ll[i].1) else {
            return false;
        };
        pts[i] = p;
    }
    if !globe_quad_screen_ok(pts, max_edge) {
        return false;
    }
    let mut mesh = Mesh::with_texture(tid);
    for i in 0..4 {
        mesh.vertices.push(Vertex {
            pos: pts[i],
            uv: egui::pos2(uvs[i][0], uvs[i][1]),
            color: tint,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
    true
}

/// 本站全球雷达单幅：图幅为 Web Mercator 瓦片拼接，按墨卡托平面贴到平面或球体。
pub struct GeoImageOverlay {
    id: String,
    tex: Option<TextureHandle>,
    min_lon: f64,
    min_lat: f64,
    max_lon: f64,
    max_lat: f64,
    pending: Option<Vec<u8>>,
    tint: Color32,
}

impl Default for GeoImageOverlay {
    fn default() -> Self {
        Self::radar()
    }
}

impl GeoImageOverlay {
    pub fn radar() -> Self {
        Self::with_tint(Color32::from_rgba_unmultiplied(255, 255, 255, 200))
    }

    pub fn cloud() -> Self {
        // 红外彩色已含透明；略提不透明度以免暗底图上发灰
        Self::with_tint(Color32::from_rgba_unmultiplied(255, 255, 255, 230))
    }

    pub fn with_tint(tint: Color32) -> Self {
        Self {
            id: String::new(),
            tex: None,
            min_lon: 0.0,
            min_lat: 0.0,
            max_lon: 0.0,
            max_lat: 0.0,
            pending: None,
            tint,
        }
    }

    pub fn set_source(&mut self, sheet: Option<&CloudSheet>) {
        let tint = self.tint;
        let Some(s) = sheet else {
            *self = Self::with_tint(tint);
            return;
        };
        if s.id == self.id {
            return;
        }
        self.id = s.id.clone();
        self.min_lon = s.min_lon;
        self.min_lat = s.min_lat;
        self.max_lon = s.max_lon;
        self.max_lat = s.max_lat;
        self.tex = None;
        self.pending = Some(s.png.clone());
        self.tint = tint;
    }

    /// 图像 UV（北在上）→ 经纬。u/v 在墨卡托平面线性，与服务端拼图几何一致。
    fn uv_to_lonlat(&self, u: f64, v: f64) -> (f64, f64) {
        const Z: f64 = 0.0;
        let (wx0, wy0) = projection::lonlat_to_world(self.min_lon, self.max_lat, Z);
        let (wx1, wy1) = projection::lonlat_to_world(self.max_lon, self.min_lat, Z);
        let wx = wx0 + (wx1 - wx0) * u;
        let wy = wy0 + (wy1 - wy0) * v;
        projection::world_to_lonlat(wx, wy, Z)
    }

    pub fn paint(&mut self, ui: &mut Ui, view: &ViewCtx) {
        if self.tex.is_none() {
            if let Some(png) = self.pending.take() {
                if let Some(img) = decode_rgba(&png) {
                    self.tex = Some(ui.ctx().load_texture(
                        format!("wx-sheet-v2-{}", self.id),
                        img,
                        TextureOptions::LINEAR,
                    ));
                }
            }
        }
        let Some(tex) = &self.tex else {
            return;
        };
        if !(self.min_lon < self.max_lon && self.min_lat < self.max_lat) {
            return;
        }
        let tid = tex.id();
        let painter = ui.painter_at(view.rect);
        let tint = self.tint;

        // 平面：整幅按墨卡托世界像素对齐底图，并画 ±1 世界副本以跨日界线。
        if !view.mode.is_globe() {
            let origin = view.rect.center();
            let z = view.zoom;
            let (cx, cy) = projection::lonlat_to_world(view.center_lon, view.center_lat, z);
            let (wx0, wy0) = projection::lonlat_to_world(self.min_lon, self.max_lat, z);
            let (wx1, wy1) = projection::lonlat_to_world(self.max_lon, self.min_lat, z);
            let world = projection::world_size(z);
            let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
            for k in -1i32..=1 {
                let shift = k as f64 * world;
                let tile_rect = Rect::from_min_max(
                    Pos2::new(
                        origin.x + ((wx0 + shift - cx) as f32),
                        origin.y + ((wy0 - cy) as f32),
                    ),
                    Pos2::new(
                        origin.x + ((wx1 + shift - cx) as f32),
                        origin.y + ((wy1 - cy) as f32),
                    ),
                );
                if tile_rect.intersects(view.rect) {
                    painter.image(tid, tile_rect, uv, tint);
                }
            }
            return;
        }

        // 球体：更密贴面，避免高分辨率云图在球上发糊/出现整齐缺口感。
        let max_edge = (view.rect.width().min(view.rect.height()) * 0.032).clamp(18.0, 40.0);
        const NX: i32 = 96;
        const NY: i32 = 56;
        for j in 0..NY {
            for i in 0..NX {
                let u0 = i as f64 / NX as f64;
                let u1 = (i + 1) as f64 / NX as f64;
                let v0 = j as f64 / NY as f64;
                let v1 = (j + 1) as f64 / NY as f64;
                paint_globe_cell(&painter, view, self, tid, tint, u0, u1, v0, v1, 0, max_edge);
            }
        }
    }
}

fn paint_globe_cell(
    painter: &egui::Painter,
    view: &ViewCtx,
    sheet: &GeoImageOverlay,
    tid: egui::TextureId,
    tint: Color32,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    depth: u8,
    max_edge: f32,
) {
    let corners = [
        sheet.uv_to_lonlat(u0, v0),
        sheet.uv_to_lonlat(u1, v0),
        sheet.uv_to_lonlat(u1, v1),
        sheet.uv_to_lonlat(u0, v1),
    ];
    let n_ok = corners
        .iter()
        .filter(|(lon, lat)| view.project(*lon, *lat).is_some())
        .count();
    if n_ok == 0 {
        return;
    }

    // 中心与边中点也须朝向相机，否则四角可见时弦仍可能切穿球体
    let samples = [
        ((u0 + u1) * 0.5, (v0 + v1) * 0.5),
        ((u0 + u1) * 0.5, v0),
        ((u0 + u1) * 0.5, v1),
        (u0, (v0 + v1) * 0.5),
        (u1, (v0 + v1) * 0.5),
    ];
    let mid_ok = samples.iter().all(|&(u, v)| {
        let (lon, lat) = sheet.uv_to_lonlat(u, v);
        view.project(lon, lat).is_some()
    });

    if n_ok == 4 && mid_ok {
        let painted = paint_globe_quad(
            painter,
            view,
            tid,
            tint,
            corners,
            [
                [u0 as f32, v0 as f32],
                [u1 as f32, v0 as f32],
                [u1 as f32, v1 as f32],
                [u0 as f32, v1 as f32],
            ],
            max_edge,
        );
        if painted {
            return;
        }
        // 弦过长：继续细分
    }

    if depth >= 4 {
        return;
    }
    let um = (u0 + u1) * 0.5;
    let vm = (v0 + v1) * 0.5;
    paint_globe_cell(painter, view, sheet, tid, tint, u0, um, v0, vm, depth + 1, max_edge);
    paint_globe_cell(painter, view, sheet, tid, tint, um, u1, v0, vm, depth + 1, max_edge);
    paint_globe_cell(painter, view, sheet, tid, tint, u0, um, vm, v1, depth + 1, max_edge);
    paint_globe_cell(painter, view, sheet, tid, tint, um, u1, vm, v1, depth + 1, max_edge);
}

fn decode_rgba(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

fn fetch_tile(url: &str) -> Option<ColorImage> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent("EEWView/0.1")
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_uv_matches_world_midpoints() {
        let sheet = GeoImageOverlay {
            id: "t".into(),
            tex: None,
            min_lon: -180.0,
            min_lat: -85.05112878,
            max_lon: 180.0,
            max_lat: 85.05112878,
            pending: None,
            tint: Color32::WHITE,
        };
        let (lon, lat) = sheet.uv_to_lonlat(0.5, 0.5);
        assert!(lon.abs() < 1e-6, "lon {lon}");
        assert!(lat.abs() < 1e-6, "lat {lat}");
        let (lon0, lat0) = sheet.uv_to_lonlat(0.0, 0.0);
        assert!((lon0 + 180.0).abs() < 1e-6);
        assert!((lat0 - 85.05112878).abs() < 1e-4);
    }
}
