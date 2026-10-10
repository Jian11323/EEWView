//! JMA2001 / ak135 走时表加载与插值。

use jian_util::haversine_km;
use serde::Deserialize;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TravelError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("表为空或网格无效")]
    InvalidTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    P = 0,
    S = 1,
}

/// S 波最大传播距离（km）。对齐 eewcn `maxDistanceSWaveSpread(magnitude, depth)`（depth 未参与计算）。
pub fn max_s_wave_spread_km(magnitude: f64) -> f64 {
    if magnitude <= 3.0 {
        600.0
    } else if magnitude <= 4.0 {
        800.0
    } else if magnitude <= 5.0 {
        1500.0
    } else if magnitude <= 5.5 {
        2000.0
    } else if magnitude <= 6.0 {
        2250.0
    } else if magnitude <= 6.2 {
        2500.0
    } else if magnitude <= 6.5 {
        3200.0
    } else if magnitude <= 7.0 {
        4000.0
    } else if magnitude <= 7.2 {
        5000.0
    } else if magnitude <= 7.5 {
        6000.0
    } else {
        6500.0
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TravelTable {
    pub name: String,
    pub depths_km: Vec<f64>,
    pub distances_km: Vec<f64>,
    /// [depth_idx][dist_idx]
    pub p_times_s: Vec<Vec<f64>>,
    pub s_times_s: Vec<Vec<f64>>,
}

impl TravelTable {
    pub fn load_json(path: impl AsRef<Path>) -> Result<Self, TravelError> {
        let raw = std::fs::read_to_string(path)?;
        let t: Self = serde_json::from_str(&raw)?;
        if t.depths_km.is_empty() || t.distances_km.is_empty() {
            return Err(TravelError::InvalidTable);
        }
        Ok(t)
    }

    fn grid<'a>(&'a self, wave: Wave) -> &'a [Vec<f64>] {
        match wave {
            Wave::P => &self.p_times_s,
            Wave::S => &self.s_times_s,
        }
    }

    /// 双线性插值走时（秒）
    pub fn travel_time(&self, wave: Wave, depth_km: f64, distance_km: f64) -> Option<f64> {
        let di = bracket(&self.depths_km, depth_km)?;
        let dj = bracket(&self.distances_km, distance_km)?;
        let g = self.grid(wave);
        let (d0, d1) = (self.depths_km[di.0], self.depths_km[di.1]);
        let (x0, x1) = (self.distances_km[dj.0], self.distances_km[dj.1]);
        let td = if (d1 - d0).abs() < 1e-9 {
            0.0
        } else {
            (depth_km - d0) / (d1 - d0)
        };
        let tx = if (x1 - x0).abs() < 1e-9 {
            0.0
        } else {
            (distance_km - x0) / (x1 - x0)
        };
        let v00 = g[di.0][dj.0];
        let v01 = g[di.0][dj.1];
        let v10 = g[di.1][dj.0];
        let v11 = g[di.1][dj.1];
        let v0 = v00 + (v01 - v00) * tx;
        let v1 = v10 + (v11 - v10) * tx;
        Some(v0 + (v1 - v0) * td)
    }

    /// 给定走时求表面距离（km），在深度行上对距离一维插值
    pub fn distance_for_time(&self, wave: Wave, depth_km: f64, sec: f64) -> Option<f64> {
        let di = bracket(&self.depths_km, depth_km)?;
        let g = self.grid(wave);
        let row = &g[di.0];
        if sec <= row[0] {
            return Some(self.distances_km[0]);
        }
        for i in 0..row.len().saturating_sub(1) {
            if row[i + 1] >= sec {
                let t0 = row[i];
                let t1 = row[i + 1];
                let x0 = self.distances_km[i];
                let x1 = self.distances_km[i + 1];
                if (t1 - t0).abs() < 1e-9 {
                    return Some(x0);
                }
                return Some(x0 + (sec - t0) / (t1 - t0) * (x1 - x0));
            }
        }
        None
    }
}

fn bracket(xs: &[f64], v: f64) -> Option<(usize, usize)> {
    if xs.is_empty() {
        return None;
    }
    if v <= xs[0] {
        return Some((0, 0));
    }
    if v >= xs[xs.len() - 1] {
        let i = xs.len() - 1;
        return Some((i, i));
    }
    for i in 0..xs.len() - 1 {
        if xs[i] <= v && v <= xs[i + 1] {
            return Some((i, i + 1));
        }
    }
    None
}

/// 走时引擎：近距 JMA2001，远距可选 ak135，最后直线兜底
pub struct TravelEngine {
    pub primary: TravelTable,
    pub fallback: Option<TravelTable>,
    pub linear_last_resort: bool,
}

fn resolve_table_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let n = name.trim().to_ascii_lowercase();
    let stem = n
        .strip_suffix(".json")
        .unwrap_or(&n)
        .replace("_stub", "");
    let candidates = [
        dir.join(format!("{stem}.json")),
        dir.join(format!("{stem}_stub.json")),
        dir.join(format!("{n}.json")),
    ];
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    candidates[0].clone()
}

impl TravelEngine {
    pub fn load_default(assets_travel: impl AsRef<Path>) -> Result<Self, TravelError> {
        Self::load_named(assets_travel, "jma2001", "ak135", true)
    }

    /// 按配置名加载：`jma2001` / `ak135`（文件可为 `ak135.json` 或 `ak135_stub.json`）。
    pub fn load_named(
        assets_travel: impl AsRef<Path>,
        primary: &str,
        fallback: &str,
        linear_last_resort: bool,
    ) -> Result<Self, TravelError> {
        let dir = assets_travel.as_ref();
        let primary = TravelTable::load_json(resolve_table_path(dir, primary))?;
        let fallback = TravelTable::load_json(resolve_table_path(dir, fallback)).ok();
        Ok(Self {
            primary,
            fallback,
            linear_last_resort,
        })
    }

    pub fn travel_time(&self, wave: Wave, depth_km: f64, distance_km: f64) -> f64 {
        if let Some(t) = self.primary.travel_time(wave, depth_km, distance_km) {
            return t;
        }
        if let Some(fb) = &self.fallback {
            if let Some(t) = fb.travel_time(wave, depth_km, distance_km) {
                return t;
            }
        }
        if !self.linear_last_resort {
            return f64::INFINITY;
        }
        // 最后兜底：P≈7 S≈5 km/s 斜距
        let v = match wave {
            Wave::P => 7.0,
            Wave::S => 5.0,
        };
        (depth_km.hypot(distance_km)) / v
    }

    pub fn eta_s_to_site(
        &self,
        lon: f64,
        lat: f64,
        depth_km: f64,
        site_lon: f64,
        site_lat: f64,
    ) -> f64 {
        let d = haversine_km(lon, lat, site_lon, site_lat);
        self.travel_time(Wave::S, depth_km, d)
    }

    pub fn surface_distance_for_elapsed(&self, wave: Wave, depth_km: f64, elapsed_s: f64) -> f64 {
        if let Some(d) = self.primary.distance_for_time(wave, depth_km, elapsed_s) {
            return d;
        }
        if let Some(fb) = &self.fallback {
            if let Some(d) = fb.distance_for_time(wave, depth_km, elapsed_s) {
                return d;
            }
        }
        if !self.linear_last_resort {
            return 0.0;
        }
        let v = match wave {
            Wave::P => 7.0,
            Wave::S => 5.0,
        };
        (elapsed_s * v).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_spread_matches_eewcn() {
        assert_eq!(max_s_wave_spread_km(2.5), 600.0);
        assert_eq!(max_s_wave_spread_km(3.0), 600.0);
        assert_eq!(max_s_wave_spread_km(3.1), 800.0);
        assert_eq!(max_s_wave_spread_km(5.5), 2000.0);
        assert_eq!(max_s_wave_spread_km(6.2), 2500.0);
        assert_eq!(max_s_wave_spread_km(7.5), 6000.0);
        assert_eq!(max_s_wave_spread_km(8.0), 6500.0);
    }
}
