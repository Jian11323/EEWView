//! 公共小工具：地理距离、角度换算等。

/// 地球平均半径（km）
pub const EARTH_RADIUS_KM: f64 = 6371.0;

#[inline]
pub fn deg_to_rad(deg: f64) -> f64 {
    deg * std::f64::consts::PI / 180.0
}

/// 大圆表面距离（km）
pub fn haversine_km(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let (φ1, φ2) = (deg_to_rad(lat1), deg_to_rad(lat2));
    let dφ = deg_to_rad(lat2 - lat1);
    let dλ = deg_to_rad(lon2 - lon1);
    let a = (dφ / 2.0).sin().powi(2) + φ1.cos() * φ2.cos() * (dλ / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}
