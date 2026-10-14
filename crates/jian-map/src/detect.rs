//! 要石 `NiedNet` 风格检知：Yahoo/強震モニタ测站邻接 + activity 链激活 + 0.1° 网格方框。
//!
//! 仅处理 `kmoni:` 前缀测站（JMA 四选一网槽）；S-Net / KMA 不参与。

use egui::Color32;
use jian_core::{IntensityKind, StationSample};
use jian_util::haversine_km;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::projection::ViewCtx;

/// 激活保持（对齐要石 `NiedStation.setActive` 10.5s）
const ACTIVE_HOLD_MS: i64 = 10_500;
/// 方框半边（要石 rectangle ±0.495°）
const GRID_HALF_DEG: f64 = 0.495;
/// 邻站搜索上限
const NEARBY_LEN: usize = 6;
/// 邻站主半径 / 候选半径（km）
const NEAR_KM: f64 = 30.0;
const CANDIDATE_KM: f64 = 40.0;
/// 检知灵敏度：1=标准 2=高 3=极高（对齐要石 niedSensitivity）
const SENSITIVITY: u8 = 1;
const ACTIVITY_THRES: [f64; 7] = [f64::INFINITY, 9.0, 12.0, 14.0, 15.0, 16.0, 16.0];

#[derive(Debug, Clone, Copy)]
pub struct DetectBox {
    pub lon: f64,
    pub lat: f64,
    /// Yahoo/要石色带档 0–20
    pub band: u8,
}

struct Sta {
    id: String,
    lat: f64,
    lon: f64,
    level: i8,
    recent: VecDeque<i8>,
    ascend: i32,
    activity: f64,
    is_active: bool,
    active_until: i64,
    default_expire: i32,
    expire: i32,
}

#[derive(Default)]
pub struct DetectTracker {
    order: Vec<String>,
    by_id: HashMap<String, Sta>,
    adj: HashMap<String, Vec<String>>,
    dist_km: HashMap<(String, String), f64>,
    decimal: [f64; 2],
    net_was_active: bool,
    last_max_audio: Option<u8>,
    last_fp: u64,
    last_tick_ms: i64,
    cached_boxes: Vec<DetectBox>,
}

impl DetectTracker {
    /// 更新激活集；返回网格方框，以及震度播报档（0–7）升档时的档位。
    pub fn tick(
        &mut self,
        stations: &[StationSample],
        now_ms: i64,
    ) -> (Vec<DetectBox>, Option<u8>) {
        // 仅 JMA 网槽（merge 后 id 带 kmoni:）
        let jma: Vec<&StationSample> = stations
            .iter()
            .filter(|s| {
                s.id.starts_with("kmoni:")
                    && s.intensity_kind == IntensityKind::JmaShindo
                    && (s.latitude.abs() > 1e-6 || s.longitude.abs() > 1e-6)
            })
            .collect();

        if jma.is_empty() {
            if self.by_id.is_empty() {
                return (Vec::new(), None);
            }
            // 无新帧：只到期熄灭
            self.expire_actives(now_ms);
            let boxes = self.build_boxes();
            self.cached_boxes = boxes.clone();
            return (boxes, None);
        }

        let fp = frame_fingerprint(&jma);
        let same_frame = fp == self.last_fp;
        if same_frame
            && now_ms.saturating_sub(self.last_tick_ms) < 200
            && !self.net_was_active
        {
            return (self.cached_boxes.clone(), None);
        }
        self.last_tick_ms = now_ms;

        if !same_frame {
            self.last_fp = fp;
            self.sync_topology(&jma);
            self.apply_levels(&jma);
        }
        self.expire_actives(now_ms);
        if !same_frame {
            self.run_activation(now_ms);
        }

        let boxes = self.build_boxes();
        let max_audio = max_shindo_class(&boxes);
        let rose = match (max_audio, self.last_max_audio) {
            (Some(cur), Some(prev)) if cur > prev => Some(cur),
            (Some(cur), None) if cur >= 1 => Some(cur),
            _ => None,
        };
        if boxes.is_empty() {
            self.last_max_audio = None;
            self.net_was_active = false;
        } else if let Some(m) = max_audio {
            self.last_max_audio = Some(self.last_max_audio.map(|p| p.max(m)).unwrap_or(m));
            self.net_was_active = true;
        }

        self.cached_boxes = boxes.clone();
        (boxes, rose)
    }

    fn sync_topology(&mut self, jma: &[&StationSample]) {
        let mut order: Vec<String> = jma.iter().map(|s| s.id.clone()).collect();
        order.sort();
        let same = order == self.order
            && order.iter().all(|id| {
                self.by_id.get(id).is_some_and(|st| {
                    jma.iter().any(|s| {
                        s.id == *id
                            && (s.latitude - st.lat).abs() < 1e-4
                            && (s.longitude - st.lon).abs() < 1e-4
                    })
                })
            });
        if same {
            return;
        }

        self.order = order;
        self.adj.clear();
        self.dist_km.clear();
        let mut next: HashMap<String, Sta> = HashMap::with_capacity(jma.len());
        for s in jma {
            let old = self.by_id.remove(&s.id);
            next.insert(
                s.id.clone(),
                Sta {
                    id: s.id.clone(),
                    lat: s.latitude,
                    lon: s.longitude,
                    level: old.as_ref().map(|o| o.level).unwrap_or(-1),
                    recent: old.map(|o| o.recent).unwrap_or_default(),
                    ascend: 0,
                    activity: 0.0,
                    is_active: false,
                    active_until: 0,
                    default_expire: 5,
                    expire: 5,
                },
            );
        }
        self.by_id = next;
        self.rebuild_adjacency();
    }

    fn rebuild_adjacency(&mut self) {
        let ids: Vec<String> = self.order.clone();
        let n = ids.len();
        for i in 0..n {
            let a = &ids[i];
            let sa = &self.by_id[a];
            let mut within: Vec<(String, f64)> = Vec::new();
            let mut candidate: Option<(String, f64)> = None;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let b = &ids[j];
                let sb = &self.by_id[b];
                let d = if let Some(v) = self.dist_km.get(&(b.clone(), a.clone())) {
                    *v
                } else {
                    let v = haversine_km(sa.lon, sa.lat, sb.lon, sb.lat);
                    self.dist_km.insert((a.clone(), b.clone()), v);
                    self.dist_km.insert((b.clone(), a.clone()), v);
                    v
                };
                if d <= NEAR_KM {
                    within.push((b.clone(), d));
                } else if d <= CANDIDATE_KM {
                    let better = match candidate {
                        None => true,
                        Some((_, cd)) => d <= cd,
                    };
                    if better {
                        candidate = Some((b.clone(), d));
                    }
                }
            }
            if within.len() <= 1 {
                if let Some(c) = candidate {
                    within.push(c);
                }
            }
            within.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
            within.truncate(NEARBY_LEN);
            let max_d = within.last().map(|(_, d)| *d).unwrap_or(0.0);
            let exp = ((max_d / 3.5).round() as i32).max(5);
            if let Some(st) = self.by_id.get_mut(a) {
                st.default_expire = exp;
                st.expire = exp;
            }
            self.adj
                .insert(a.clone(), within.into_iter().map(|(id, _)| id).collect());
        }
    }

    fn apply_levels(&mut self, jma: &[&StationSample]) {
        for s in jma {
            let Some(st) = self.by_id.get_mut(&s.id) else {
                continue;
            };
            let origin = level_from_instrumental(s.instrumental);
            let level = if origin == -1 {
                st.recent
                    .iter()
                    .take(4)
                    .copied()
                    .find(|&v| v != -1)
                    .unwrap_or(-1)
            } else {
                origin
            };
            if level > st.level && st.level != -1 {
                st.expire = (st.expire + 2).min(30);
            } else if level < st.level || level == -1 {
                st.expire = st.default_expire;
            }
            st.level = level;
            let recent_filter: Vec<i8> = st
                .recent
                .iter()
                .take(st.expire as usize)
                .copied()
                .filter(|&v| v != -1)
                .collect();
            let ascend = if let Some(&min_r) = recent_filter.iter().min() {
                i32::from(level) - i32::from(min_r)
            } else {
                0
            };
            st.ascend = ascend;
            st.activity = calc_activity(level, ascend, st.is_active);
            st.recent.push_front(origin);
            while st.recent.len() > 30 {
                st.recent.pop_back();
            }
            if st.expire > st.default_expire
                && !st.is_active
                && st.recent.len() >= st.expire as usize
            {
                let slice: Vec<i8> = st
                    .recent
                    .iter()
                    .take(st.expire as usize)
                    .copied()
                    .filter(|&v| v != -1)
                    .collect();
                if !slice.is_empty() && slice.iter().all(|&v| v == slice[0]) {
                    st.expire = st.default_expire;
                }
            }
        }
    }

    fn expire_actives(&mut self, now_ms: i64) {
        for st in self.by_id.values_mut() {
            if st.is_active && now_ms >= st.active_until {
                st.is_active = false;
            }
        }
    }

    fn run_activation(&mut self, now_ms: i64) {
        let possible: Vec<String> = self
            .by_id
            .values()
            .filter(|s| s.activity > 0.0)
            .map(|s| s.id.clone())
            .collect();
        let mut active: HashSet<String> = HashSet::new();
        let mut checked: HashSet<String> = HashSet::new();

        for id in &possible {
            if checked.contains(id) {
                continue;
            }
            let Some(st) = self.by_id.get(id) else {
                continue;
            };
            if st.is_active && st.ascend > 0 {
                self.chain_activate(id, &mut active, &mut checked);
                continue;
            }
            let neighbors: Vec<String> = self
                .adj
                .get(id)
                .into_iter()
                .flatten()
                .filter(|nid| self.by_id.get(*nid).is_some_and(|n| n.level > -1))
                .cloned()
                .collect();
            let possible_near: Vec<&Sta> = neighbors
                .iter()
                .filter_map(|nid| self.by_id.get(nid))
                .filter(|n| n.activity > 0.0)
                .collect();
            let half_penalty = possible_near
                .iter()
                .filter(|n| n.ascend <= 1 && !n.is_active)
                .count() as f64
                / 2.0;
            let nearby_active_num = possible_near.len() as f64 - half_penalty;
            let n_near = neighbors.len();
            let (num_thres, activity_thres) = sens_thres(n_near);
            if nearby_active_num >= num_thres {
                let num_activity = nearby_active_num * (nearby_active_num + 1.0) / 2.0;
                let mut nearby_activity = num_activity;
                for (idx, nid) in neighbors.iter().enumerate() {
                    let Some(n) = self.by_id.get(nid) else {
                        continue;
                    };
                    let d = self
                        .dist_km
                        .get(&(id.clone(), nid.clone()))
                        .copied()
                        .unwrap_or(0.0);
                    if idx >= 3 && d > 15.0 {
                        nearby_activity += n.activity / 2.0;
                    } else {
                        nearby_activity += n.activity;
                    }
                }
                if nearby_activity >= activity_thres {
                    self.chain_activate(id, &mut active, &mut checked);
                }
            } else {
                checked.insert(id.clone());
            }
        }

        if !self.net_was_active {
            let mut best: Option<(String, i8)> = None;
            for id in &active {
                if let Some(st) = self.by_id.get(id) {
                    let better = match best {
                        None => true,
                        Some((_, lv)) => st.level > lv,
                    };
                    if better {
                        best = Some((id.clone(), st.level));
                    }
                }
            }
            if let Some((id, _)) = best {
                if let Some(st) = self.by_id.get(&id) {
                    self.decimal = [
                        frac_decimal(st.lat),
                        frac_decimal(st.lon),
                    ];
                }
            }
        }

        for id in active {
            if let Some(st) = self.by_id.get_mut(&id) {
                st.is_active = true;
                st.active_until = now_ms + ACTIVE_HOLD_MS;
            }
        }
    }

    fn chain_activate(
        &self,
        start: &str,
        active: &mut HashSet<String>,
        checked: &mut HashSet<String>,
    ) {
        let mut pending = vec![start.to_string()];
        while let Some(cur) = pending.pop() {
            if !checked.insert(cur.clone()) {
                continue;
            }
            let Some(st) = self.by_id.get(&cur) else {
                continue;
            };
            if st.activity > 0.0 {
                active.insert(cur.clone());
                if let Some(neis) = self.adj.get(&cur) {
                    for n in neis {
                        if !checked.contains(n) {
                            pending.push(n.clone());
                        }
                    }
                }
            }
        }
    }

    fn build_boxes(&self) -> Vec<DetectBox> {
        let mut grids: HashMap<(i32, i32), u8> = HashMap::new();
        for st in self.by_id.values() {
            if !st.is_active || st.level < 0 {
                continue;
            }
            let lat = snap(st.lat, self.decimal[0]);
            let lon = snap(st.lon, self.decimal[1]);
            let key = (
                (lat * 1000.0).round() as i32,
                (lon * 1000.0).round() as i32,
            );
            let band = st.level.clamp(0, 20) as u8;
            grids
                .entry(key)
                .and_modify(|b| *b = (*b).max(band))
                .or_insert(band);
        }
        grids
            .into_iter()
            .map(|((lat_i, lon_i), band)| DetectBox {
                lat: f64::from(lat_i) / 1000.0,
                lon: f64::from(lon_i) / 1000.0,
                band,
            })
            .collect()
    }
}

fn frame_fingerprint(jma: &[&StationSample]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    jma.len().hash(&mut h);
    let mut acc = 0u64;
    let mut max_bits = 0u64;
    for s in jma {
        let b = s.instrumental.to_bits();
        acc = acc.wrapping_add(b);
        max_bits = max_bits.max(b);
    }
    acc.hash(&mut h);
    max_bits.hash(&mut h);
    h.finish()
}

fn level_from_instrumental(v: f64) -> i8 {
    if !v.is_finite() || v < -3.0 {
        return -1;
    }
    let band = ((v + 3.0) * 2.0).round() as i32;
    band.clamp(0, 20) as i8
}

fn calc_activity(level: i8, ascend: i32, is_active: bool) -> f64 {
    let level_activity = if ascend > 0 || is_active {
        if level <= 5 {
            0.0
        } else if level <= 7 {
            if is_active {
                0.5 * f64::from(level - 5)
            } else {
                0.25 * f64::from(level - 5)
            }
        } else if level <= 11 {
            2.0 * f64::from(level - 7)
        } else {
            6.0 * f64::from(level - 10)
        }
    } else {
        0.0
    };
    let ascend_activity = if ascend <= 0 {
        0.0
    } else if ascend <= 1 {
        if is_active {
            0.5
        } else {
            0.25
        }
    } else if ascend <= 6 {
        2.0 * f64::from(ascend - 2) + 1.0
    } else {
        6.0 * f64::from(ascend - 5)
    };
    level_activity + ascend_activity
}

fn sens_thres(n_near: usize) -> (f64, f64) {
    let idx = n_near.min(6);
    let base = ACTIVITY_THRES[idx];
    match SENSITIVITY {
        2 => {
            let num = if n_near <= 2 {
                (n_near as f64 + 1.0) / 2.0
            } else {
                n_near as f64 / 2.0
            };
            (num, base)
        }
        3 => (n_near as f64 / 2.0, base - 2.0),
        _ => (3.0, base + 2.0), // 标准
    }
}

fn frac_decimal(v: f64) -> f64 {
    // Math.round((val + 180) % 1 * 10) / 10
    let frac = (v + 180.0).rem_euclid(1.0);
    (frac * 10.0).round() / 10.0
}

fn snap(v: f64, decimal: f64) -> f64 {
    (v - decimal).round() + decimal
}

/// 要石 `currentMaxShindo`：色带档 → 播报 0–7。
fn max_shindo_class(boxes: &[DetectBox]) -> Option<u8> {
    let max_level = boxes.iter().map(|b| b.band).max()?;
    Some(shindo_from_level(max_level))
}

fn shindo_from_level(level: u8) -> u8 {
    if level <= 7 {
        0
    } else if level <= 9 {
        1
    } else if level <= 11 {
        2
    } else if level <= 13 {
        3
    } else if level <= 15 {
        4
    } else if level <= 17 {
        5
    } else if level <= 19 {
        6
    } else {
        7
    }
}

fn box_color(band: u8) -> Color32 {
    // 对齐要石：≤7 绿 / ≤13 黄 / 更红
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
            (b.lon - GRID_HALF_DEG, b.lat + GRID_HALF_DEG),
            (b.lon + GRID_HALF_DEG, b.lat + GRID_HALF_DEG),
            (b.lon + GRID_HALF_DEG, b.lat - GRID_HALF_DEG),
            (b.lon - GRID_HALF_DEG, b.lat - GRID_HALF_DEG),
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
