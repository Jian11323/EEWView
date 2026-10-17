//! 气象叠层 XYZ 瓦片（RainViewer 等），与底图缓存分离。

use crate::projection::{self, TILE_SIZE};
use egui::{Color32, ColorImage, Context, Rect, TextureHandle, TextureOptions, Ui};
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
        while let Ok(decoded) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
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
    }

    pub fn paint(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        center_lon: f64,
        center_lat: f64,
        zoom: f64,
    ) {
        if self.prefix.is_empty() {
            return;
        }
        self.poll(ui.ctx());
        let z = zoom.floor().clamp(0.0, 7.0) as u8;
        let zf = z as f64;
        let scale = (2f64.powf(zoom - zf)) as f32;
        let tile_px = (TILE_SIZE as f32) * scale;
        let (_, x0, y0, x1, y1) =
            projection::visible_tiles(center_lon, center_lat, zoom, rect, 0);
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
        let painter = ui.painter_at(rect);
        let (cx, cy) = projection::lonlat_to_world(center_lon, center_lat, zf);
        let origin = rect.center();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let id = TileId {
                    z,
                    x: projection::wrap_tile_x(x, z),
                    y: y as u32,
                };
                let (twx, twy) = projection::tile_world_origin(x, y, z);
                let screen_min = egui::pos2(
                    origin.x + ((twx - cx) as f32) * scale,
                    origin.y + ((twy - cy) as f32) * scale,
                );
                let tile_rect = Rect::from_min_size(screen_min, egui::vec2(tile_px, tile_px));
                if let Some(TileState::Ready(tex)) = self.entries.get(&id) {
                    painter.image(
                        tex.id(),
                        tile_rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
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
