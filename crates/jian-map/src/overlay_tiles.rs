//! 气象叠层 XYZ 瓦片（RainViewer 等），与底图缓存分离。

use crate::projection::{self, ViewCtx, TILE_SIZE};
use egui::{
    Color32, ColorImage, Context, Mesh, Pos2, Rect, TextureHandle, TextureOptions, Ui,
};
use egui::epaint::Vertex;
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
        }
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
        let z = zoom.floor().clamp(0.0, 7.0) as u8;
        let zf = z as f64;
        let scale = (2f64.powf(zoom - zf)) as f32;
        let tile_px = (TILE_SIZE as f32) * scale;
        let (_, x0, y0, x1, y1) = projection::visible_tiles(
            view.center_lon,
            view.center_lat,
            zoom,
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
                    // 四角经纬投影为纹理四边形；少于 3 角在地平上则跳过
                    let (ox, oy) = projection::tile_world_origin(x, y, z);
                    let (ox1, oy1) = projection::tile_world_origin(x + 1, y + 1, z);
                    let corners_ll = [
                        projection::world_to_lonlat(ox, oy, zf),
                        projection::world_to_lonlat(ox1, oy, zf),
                        projection::world_to_lonlat(ox1, oy1, zf),
                        projection::world_to_lonlat(ox, oy1, zf),
                    ];
                    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                    let mut pts = [None; 4];
                    let mut n_ok = 0usize;
                    for (i, (lon, lat)) in corners_ll.iter().enumerate() {
                        pts[i] = view.project(*lon, *lat);
                        if pts[i].is_some() {
                            n_ok += 1;
                        }
                    }
                    if n_ok < 3 {
                        continue;
                    }
                    let mut mesh = Mesh::with_texture(tid);
                    for i in 0..4 {
                        let pos = pts[i].unwrap_or_else(|| {
                            // 缺角用对角邻点中点近似，避免拉到视口中心
                            let a = pts[(i + 3) % 4];
                            let b = pts[(i + 1) % 4];
                            match (a, b) {
                                (Some(pa), Some(pb)) => Pos2::new(
                                    (pa.x + pb.x) * 0.5,
                                    (pa.y + pb.y) * 0.5,
                                ),
                                (Some(pa), None) | (None, Some(pa)) => pa,
                                _ => origin,
                            }
                        });
                        mesh.vertices.push(Vertex {
                            pos,
                            uv: egui::pos2(uvs[i][0], uvs[i][1]),
                            color: self.tint,
                        });
                    }
                    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                    painter.add(egui::Shape::mesh(mesh));
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
