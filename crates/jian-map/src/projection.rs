//! Web Mercator 投影（XYZ 瓦片坐标系；经度相对中心折到 ±180°，支持循环地图）。

use egui::{Pos2, Rect, Vec2};
use jian_util::EARTH_RADIUS_KM;

pub const TILE_SIZE: f64 = 256.0;

/// 将纬度钳到 Web Mercator 有效范围（约 ±85.0511°）
pub fn clamp_lat(lat: f64) -> f64 {
    lat.clamp(-85.05112878, 85.05112878)
}

/// 经度归一到 (-180, 180]，对应 Leaflet `worldCopyJump` 后的中心经度。
pub fn normalize_lon(lon: f64) -> f64 {
    let mut x = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if x <= -180.0 {
        x = 180.0;
    }
    x
}

/// 将 `lon` 折到距 `center_lon` 最短弧，避免跨日界线拉出贯穿屏幕的线段。
pub fn wrap_lon_near(lon: f64, center_lon: f64) -> f64 {
    center_lon + normalize_lon(lon - center_lon)
}

/// 缩放级别 z 下世界像素边长
pub fn world_size(zoom: f64) -> f64 {
    TILE_SIZE * 2f64.powf(zoom)
}

/// 经纬 → 世界像素坐标（原点左上，y 向下）。`lon` 可超出 ±180（循环副本）。
pub fn lonlat_to_world(lon: f64, lat: f64, zoom: f64) -> (f64, f64) {
    let lat = clamp_lat(lat);
    let w = world_size(zoom);
    let x = (lon + 180.0) / 360.0 * w;
    let sin_lat = (lat * std::f64::consts::PI / 180.0).sin();
    let y = (0.5 - (0.25 * std::f64::consts::FRAC_1_PI) * ((1.0 + sin_lat) / (1.0 - sin_lat)).ln())
        * w;
    (x, y)
}

/// 世界像素 → 经纬（经度不强制折回 ±180，便于连续平移）
pub fn world_to_lonlat(wx: f64, wy: f64, zoom: f64) -> (f64, f64) {
    let w = world_size(zoom);
    let lon = wx / w * 360.0 - 180.0;
    let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * wy / w;
    let lat = 180.0 / std::f64::consts::PI * n.sinh().atan();
    (lon, lat)
}

/// 视口中心经纬 + 视口矩形 → 屏幕像素（经度相对中心最短弧）
pub fn lonlat_to_screen(
    lon: f64,
    lat: f64,
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    rect: Rect,
) -> Pos2 {
    let lon = wrap_lon_near(lon, center_lon);
    let (cx, cy) = lonlat_to_world(center_lon, center_lat, zoom);
    let (wx, wy) = lonlat_to_world(lon, lat, zoom);
    let dx = (wx - cx) as f32;
    let dy = (wy - cy) as f32;
    rect.center() + Vec2::new(dx, dy)
}

/// 屏幕像素偏移 → 经纬中心变化（拖拽用）；结束后将经度折回 (-180, 180]
pub fn pan_by_screen_delta(
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    dx_px: f32,
    dy_px: f32,
) -> (f64, f64) {
    let (cx, cy) = lonlat_to_world(center_lon, center_lat, zoom);
    // 屏幕右拖 = 地图左移 → 中心世界坐标减小 x
    let (lon, lat) = world_to_lonlat(cx - dx_px as f64, cy - dy_px as f64, zoom);
    (normalize_lon(lon), clamp_lat(lat))
}

/// 表面距离（km）→ 屏幕像素半径（以中心纬度的赤道近似）
pub fn km_to_pixels(km: f64, lat: f64, zoom: f64) -> f32 {
    let w = world_size(zoom);
    let meters_per_pixel =
        (clamp_lat(lat).to_radians().cos() * 2.0 * std::f64::consts::PI * EARTH_RADIUS_KM * 1000.0)
            / w;
    if meters_per_pixel <= 1e-9 {
        return 0.0;
    }
    (km * 1000.0 / meters_per_pixel) as f32
}

/// 可见瓦片 XYZ 范围（含边距）。`zoom` 可为小数；瓦片层级取 `floor(zoom)`。
/// `x` 允许越界（循环地图）；`y` 钳在 `[0, n)`。
pub fn visible_tiles(
    center_lon: f64,
    center_lat: f64,
    zoom: f64,
    rect: Rect,
    pad: i32,
) -> (u8, i32, i32, i32, i32) {
    let z = zoom.floor().clamp(0.0, 18.0) as u8;
    let zf = z as f64;
    let scale = 2f64.powf(zoom - zf);
    let n = 1i32 << z;
    let (cx, cy) = lonlat_to_world(center_lon, center_lat, zf);
    // 屏幕半宽对应的 z 级世界像素（已除以放大倍率）
    let half_w = rect.width() as f64 * 0.5 / scale;
    let half_h = rect.height() as f64 * 0.5 / scale;
    let x0 = ((cx - half_w) / TILE_SIZE).floor() as i32 - pad;
    let y0 = ((cy - half_h) / TILE_SIZE).floor() as i32 - pad;
    let x1 = ((cx + half_w) / TILE_SIZE).floor() as i32 + pad;
    let y1 = ((cy + half_h) / TILE_SIZE).floor() as i32 + pad;
    (z, x0, y0.clamp(0, n - 1), x1, y1.clamp(0, n - 1))
}

/// 将可能越界的瓦片列号折到 `[0, n)`（循环取图）
pub fn wrap_tile_x(x: i32, z: u8) -> u32 {
    let n = 1i32 << z;
    (((x % n) + n) % n) as u32
}

/// 瓦片左上角在该 z 级下的世界像素坐标
pub fn tile_world_origin(x: i32, y: i32, _z: u8) -> (f64, f64) {
    (x as f64 * TILE_SIZE, y as f64 * TILE_SIZE)
}
