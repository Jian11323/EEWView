//! NIED / 要石风格检知框：有感测站邻域激活后画 ±0.5° 方框，并报告震度升档。

use egui::Color32;
use jian_core::{
    instrumental_band_level, jma_from_instrumental, IntensityKind, StationSample,
};
use std::collections::HashMap;

use crate::projection::ViewCtx;

/// 激活保持时长（对齐要石约 12.5s）
const HOLD_MS: i64 = 12_500;
/// 方框半边长（度）
const GRID_RADIUS_DEG: f64 = 0.5;
/// 邻站距离阈值（度，简化版，无 NIED 邻接表）
const NEIGHBOR_DEG: f64 = 0.75;

#[derive(Debug, Clone, Copy)]
pub struct DetectBox {
    pub lon: f64,
    pub lat: f64,
    /// 計測色带档 0–20
    pub band: u8,
}

#[derive(Debug, Default)]
pub struct DetectTracker {
    /// 测站 id → 熄灭时刻 ms
    until: HashMap<String, i64>,
    /// 本轮检出最大震度播报档（0–7）；静默后清空
    last_max_audio: Option<u8>,
    /// 无活动时降频 tick，减轻每帧邻域扫描
    last_tick_ms: i64,
    cached_boxes: Vec<DetectBox>,
}

impl DetectTracker {
    /// 更新激活集；返回本帧方框，以及若震度升档则给出 `AudioCue::Shindo` 用的 0–7 档。
    pub fn tick(
        &mut self,
        stations: &[StationSample],
        now_ms: i64,
    ) -> (Vec<DetectBox>, Option<u8>) {
        let felt_any = stations.iter().any(|s| {
            s.intensity_kind == IntensityKind::JmaShindo && s.instrumental >= 0.5
        });
        // 无有感且无保持框：约 4Hz 扫描即可
        if !felt_any && self.until.is_empty() && now_ms.saturating_sub(self.last_tick_ms) < 250 {
            return (self.cached_boxes.clone(), None);
        }

        self.last_tick_ms = now_ms;
        self.until.retain(|_, exp| *exp > now_ms);

        let felt: Vec<&StationSample> = stations
            .iter()
            .filter(|s| {
                s.intensity_kind == IntensityKind::JmaShindo && s.instrumental >= 0.5
            })
            .collect();

        for s in &felt {
            let alone = s.instrumental >= 2.5;
            let neighbors = felt
                .iter()
                .filter(|o| {
                    o.id != s.id
                        && (o.latitude - s.latitude).abs() <= NEIGHBOR_DEG
                        && shortest_lon_delta(o.longitude, s.longitude) <= NEIGHBOR_DEG
                })
                .count();
            if alone || neighbors >= 1 {
                self.until.insert(s.id.clone(), now_ms + HOLD_MS);
            }
        }

        let mut boxes = Vec::new();
        let mut max_audio: Option<u8> = None;
        for s in stations {
            if !self.until.contains_key(&s.id) {
                continue;
            }
            let band = instrumental_band_level(s.instrumental).unwrap_or(0);
            boxes.push(DetectBox {
                lon: s.longitude,
                lat: s.latitude,
                band,
            });
            if s.instrumental >= 0.5 {
                let (lv, _) = jma_from_instrumental(s.instrumental);
                let a = shindo_audio_class(lv);
                max_audio = Some(max_audio.map(|m| m.max(a)).unwrap_or(a));
            }
        }

        let rose = match (max_audio, self.last_max_audio) {
            (Some(cur), Some(prev)) if cur > prev => Some(cur),
            (Some(cur), None) if cur >= 1 => Some(cur),
            _ => None,
        };

        if boxes.is_empty() {
            self.last_max_audio = None;
        } else if let Some(m) = max_audio {
            self.last_max_audio = Some(self.last_max_audio.map(|p| p.max(m)).unwrap_or(m));
        }

        self.cached_boxes = boxes.clone();
        (boxes, rose)
    }
}

/// JMA 展示档 0–9 → 播报档 0–7（5弱/5强→5，6弱/6强→6，7→7）。
fn shindo_audio_class(level_0_9: u8) -> u8 {
    match level_0_9.min(9) {
        0..=4 => level_0_9,
        5 | 6 => 5,
        7 | 8 => 6,
        _ => 7,
    }
}

fn shortest_lon_delta(a: f64, b: f64) -> f64 {
    let mut d = (a - b).abs() % 360.0;
    if d > 180.0 {
        d = 360.0 - d;
    }
    d
}

fn box_color(band: u8) -> Color32 {
    // 对齐要石 NiedGridCanvasLayer：≤7 绿 / ≤13 黄 / 更红
    if band <= 7 {
        Color32::from_rgb(0x00, 0xc8, 0x53)
    } else if band <= 13 {
        Color32::from_rgb(0xff, 0xc1, 0x07)
    } else {
        Color32::from_rgb(0xf4, 0x43, 0x36)
    }
}

/// 绘制检知方框（只描边，不抢点击）。
pub fn paint_detect_boxes(ui: &mut egui::Ui, view: &ViewCtx, boxes: &[DetectBox]) {
    if boxes.is_empty() {
        return;
    }
    let painter = ui.painter_at(view.rect);
    let stroke_w = 2.0_f32;
    for b in boxes {
        let color = box_color(b.band);
        let corners = [
            (b.lon - GRID_RADIUS_DEG, b.lat + GRID_RADIUS_DEG),
            (b.lon + GRID_RADIUS_DEG, b.lat + GRID_RADIUS_DEG),
            (b.lon + GRID_RADIUS_DEG, b.lat - GRID_RADIUS_DEG),
            (b.lon - GRID_RADIUS_DEG, b.lat - GRID_RADIUS_DEG),
        ];
        let mut pts = Vec::with_capacity(4);
        let mut ok = true;
        for (lon, lat) in corners {
            match view.project(lon, lat) {
                Some(p) => pts.push(p),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || pts.len() < 4 {
            continue;
        }
        painter.add(egui::Shape::Path(egui::epaint::PathShape {
            points: pts,
            closed: true,
            fill: Color32::TRANSPARENT,
            stroke: egui::epaint::PathStroke::new(stroke_w, color),
        }));
    }
}
