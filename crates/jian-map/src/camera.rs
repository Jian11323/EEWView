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
    /// 位移：GQ 余弦缓动；缩放：线性时间抛物线（对齐 eewcn），中途拉远有上限，不贴 ZOOM_MIN。
    pub fn tick(&mut self, dt: f64) -> (f64, f64, f64, bool) {
        self.elapsed = (self.elapsed + dt).min(self.duration);
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        // GQ: t1 = -cos(t*PI)*0.5+0.5
        let t1 = -((t * std::f64::consts::PI).cos()) * 0.5 + 0.5;
        let lon = lerp_lon(self.start_lon, self.end_lon, t1);
        let lat = self.start_lat + (self.end_lat - self.start_lat) * t1;
        let dist_km = haversine_km(
            self.start_lat,
            self.start_lon,
            self.end_lat,
            self.end_lon,
        );
        let z0 = self.start_zoom;
        let z1 = self.end_zoom;
        let y_m = mid_flight_zoom(z0, z1, dist_km);
        // eewcn: D=2*yM-avg; a=yA+yB-2D; b=yB-yA-a; zoom=a*t^2+b*t+yA（t 线性）
        let avg = (z0 + z1) * 0.5;
        let d = 2.0 * y_m - avg;
        let a = z0 + z1 - 2.0 * d;
        let b = z1 - z0 - a;
        let zoom = if t >= 1.0 - 1e-9 {
            self.end_zoom
        } else {
            (a * t * t + b * t + z0).clamp(ZOOM_MIN, ZOOM_MAX)
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

/// 中途拉远目标 zoom：远距最多掉约 1 级，地板 3.5，避免卡在 ZOOM_MIN。
fn mid_flight_zoom(z0: f64, z1: f64, dist_km: f64) -> f64 {
    let lo = z0.min(z1);
    let avg = (z0 + z1) * 0.5;
    if dist_km < 800.0 {
        return avg;
    }
    let pull = (dist_km / 14000.0).clamp(0.0, 1.0) * 1.0;
    let mut ym = (lo - pull).max(avg - 1.0).max(lo - 1.0).max(4.5);
    if ym > avg - 0.12 {
        ym = avg - 0.12;
    }
    ym.clamp(4.5, ZOOM_MAX)
}

fn haversine_km(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    let r = 6371.0_f64;
    let dlat = (lat1 - lat0).to_radians();
    let dlon = (lon1 - lon0).to_radians();
    let a = (dlat * 0.5).sin().powi(2)
        + lat0.to_radians().cos() * lat1.to_radians().cos() * (dlon * 0.5).sin().powi(2);
    2.0 * r * a.sqrt().asin()
}

#[allow(dead_code)]
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

/// 球体构图：西太平洋震中略向东偏，减少左侧欧亚/俄罗斯占比、留出美洲侧。
/// `zoom` 越大越近；跟焦常用 9–13，故在 zoom < 11 即开始偏置。
pub fn globe_framed_center(lon: f64, lat: f64, zoom: f64) -> (f64, f64) {
    let lon = normalize_lon(lon);
    let lat = clamp_lat(lat);
    if zoom >= 11.0 {
        return (lon, lat);
    }
    let t = ((11.0 - zoom) / 8.0).clamp(0.0, 1.0);
    // 东亚—西太平洋：震中偏左，镜头中心东移
    if (95.0..170.0).contains(&lon) && (-10.0..60.0).contains(&lat) {
        let bias = 12.0 + 20.0 * t; // 约 12°–32°
        return (normalize_lon(lon + bias), lat);
    }
    // 东太平洋 / 北美西岸：略西偏，避免右侧陆地贴边被裁
    if (-170.0..-50.0).contains(&lon) && (-10.0..70.0).contains(&lat) {
        let bias = 8.0 + 14.0 * t;
        return (normalize_lon(lon - bias), lat);
    }
    (lon, lat)
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

/// 以锚点为中心筛掉过远事件，避免全球多震把镜头拉到 ZOOM_MIN。
/// `max_sep_deg`：平面近似角距（度），约 30° ≈ 3300 km。
pub fn cluster_for_fit(
    points: &[(f64, f64, f64)],
    anchor: Option<(f64, f64)>,
    max_sep_deg: f64,
) -> Vec<(f64, f64, f64)> {
    if points.is_empty() {
        return Vec::new();
    }
    let (alon, alat) = anchor.unwrap_or((points[0].0, points[0].1));
    let max_sep = max_sep_deg.max(5.0);
    let mut out: Vec<(f64, f64, f64)> = points
        .iter()
        .copied()
        .filter(|(lon, lat, _)| {
            let mut dlon = (lon - alon).abs();
            if dlon > 180.0 {
                dlon = 360.0 - dlon;
            }
            let dlat = (lat - alat).abs();
            (dlon * dlon + dlat * dlat).sqrt() <= max_sep
        })
        .collect();
    if out.is_empty() {
        // 锚点本身不在列表时，至少保留最近的一点
        if let Some(p) = points.iter().min_by(|a, b| {
            let da = {
                let mut dlon = (a.0 - alon).abs();
                if dlon > 180.0 {
                    dlon = 360.0 - dlon;
                }
                let dlat = (a.1 - alat).abs();
                dlon * dlon + dlat * dlat
            };
            let db = {
                let mut dlon = (b.0 - alon).abs();
                if dlon > 180.0 {
                    dlon = 360.0 - dlon;
                }
                let dlat = (b.1 - alat).abs();
                dlon * dlon + dlat * dlat
            };
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            out.push(*p);
        }
    }
    out
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
        let z = zoom_to_fit_radius_km(lat, r_km.max(50.0), viewport_w.min(viewport_h), 0.52);
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
