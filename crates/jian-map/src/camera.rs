//! 地图镜头：抛物线飞跃与按波圈半径适配缩放。

use crate::projection::{self, clamp_lat, normalize_lon};

const ZOOM_MIN: f64 = 2.0;
const ZOOM_MAX: f64 = 12.0;

/// 抛物线镜头飞跃（中途略拉远，两端落在震中）。
#[derive(Debug, Clone)]
pub struct CameraFlight {
    start_lon: f64,
    start_lat: f64,
    start_zoom: f64,
    end_lon: f64,
    end_lat: f64,
    end_zoom: f64,
    elapsed: f64,
    duration: f64,
}

impl CameraFlight {
    pub fn new(
        from: (f64, f64, f64),
        to: (f64, f64, f64),
        duration_s: f64,
    ) -> Self {
        let (start_lon, start_lat, start_zoom) = from;
        let (end_lon, end_lat, end_zoom) = to;
        Self {
            start_lon: normalize_lon(start_lon),
            start_lat: clamp_lat(start_lat),
            start_zoom: start_zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            end_lon: normalize_lon(end_lon),
            end_lat: clamp_lat(end_lat),
            end_zoom: end_zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            elapsed: 0.0,
            duration: duration_s.max(0.05),
        }
    }

    /// 推进 `dt` 秒；返回 (lon, lat, zoom, finished)。
    pub fn tick(&mut self, dt: f64) -> (f64, f64, f64, bool) {
        self.elapsed = (self.elapsed + dt).min(self.duration);
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        let e = ease_in_out_cubic(t);
        let lon = lerp_lon(self.start_lon, self.end_lon, e);
        let lat = self.start_lat + (self.end_lat - self.start_lat) * e;
        // 中途拉远：与位移动画共用缓动，避免末端 zoom 猛收造成「弹一下」
        let dip = 1.15 * 4.0 * e * (1.0 - e);
        let zoom_base = self.start_zoom + (self.end_zoom - self.start_zoom) * e;
        let zoom = if t >= 1.0 - 1e-9 {
            self.end_zoom
        } else {
            (zoom_base - dip).clamp(ZOOM_MIN, ZOOM_MAX)
        };
        (
            if t >= 1.0 - 1e-9 {
                self.end_lon
            } else {
                normalize_lon(lon)
            },
            if t >= 1.0 - 1e-9 {
                self.end_lat
            } else {
                clamp_lat(lat)
            },
            zoom,
            t >= 1.0 - 1e-9,
        )
    }

    pub fn target(&self) -> (f64, f64, f64) {
        (self.end_lon, self.end_lat, self.end_zoom)
    }
}

fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn lerp_lon(a: f64, b: f64, t: f64) -> f64 {
    let mut d = normalize_lon(b - a);
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    normalize_lon(a + d * t)
}

/// 使 `radius_km` 波圈大约落在视口短边的 `fill` 比例内，反推 zoom。
pub fn zoom_to_fit_radius_km(lat: f64, radius_km: f64, viewport_min_px: f32, fill: f32) -> f64 {
    let target_px = (viewport_min_px * fill.clamp(0.25, 0.95)).max(32.0);
    if radius_km <= 1.0 {
        return 8.0;
    }
    let mut lo = ZOOM_MIN;
    let mut hi = ZOOM_MAX;
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        let px = projection::km_to_pixels(radius_km, lat, mid);
        if px > target_px {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    ((lo + hi) * 0.5).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// 多震：用包围盒 + 各震 S 半径估一个合适缩放与中心。
pub fn fit_events(
    points: &[(f64, f64, f64)],
    viewport_w: f32,
    viewport_h: f32,
) -> Option<(f64, f64, f64)> {
    if points.is_empty() {
        return None;
    }
    if points.len() == 1 {
        let (lon, lat, r_km) = points[0];
        let z = zoom_to_fit_radius_km(lat, r_km.max(80.0), viewport_w.min(viewport_h), 0.72);
        return Some((normalize_lon(lon), clamp_lat(lat), z));
    }

    let mut lon_min = f64::INFINITY;
    let mut lon_max = f64::NEG_INFINITY;
    let mut lat_min = f64::INFINITY;
    let mut lat_max = f64::NEG_INFINITY;
    let mut max_r = 80.0_f64;
    let ref_lon = points[0].0;
    for &(lon, lat, r_km) in points {
        let lon = ref_lon + normalize_lon(lon - ref_lon);
        // 粗略：经度 1°≈111km·cos(lat)，纬度 1°≈111km；半径外扩
        let dlat = r_km / 111.0;
        let dlon = r_km / (111.0 * clamp_lat(lat).to_radians().cos().max(0.2));
        lon_min = lon_min.min(lon - dlon);
        lon_max = lon_max.max(lon + dlon);
        lat_min = lat_min.min(lat - dlat);
        lat_max = lat_max.max(lat + dlat);
        max_r = max_r.max(r_km);
    }
    let center_lon = normalize_lon((lon_min + lon_max) * 0.5);
    let center_lat = clamp_lat((lat_min + lat_max) * 0.5);
    let span_lat = (lat_max - lat_min).max(0.5);
    let span_lon = (lon_max - lon_min).max(0.5);
    let span_km = (span_lat * 111.0)
        .max(span_lon * 111.0 * center_lat.to_radians().cos().abs().max(0.2))
        .max(max_r * 2.0);
    let z = zoom_to_fit_radius_km(
        center_lat,
        span_km * 0.5,
        viewport_w.min(viewport_h),
        0.78,
    );
    Some((center_lon, center_lat, z))
}
