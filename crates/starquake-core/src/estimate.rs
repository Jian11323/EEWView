//! 震中最大震度/烈度估算（对齐 RhythmQuake `IntensityCalculator`）。

use crate::{cn_intensity_text, jma_from_instrumental, IntensityKind};
use starquake_util::EARTH_RADIUS_KM;
use std::f64::consts::LN_10;

/// 反解烈度环时的地表距搜索上限（km）。
const RING_SEARCH_MAX_KM: f64 = 8_000.0;
/// 仅当相邻档半径差小于此值（数值噪声）才跳过，避免重复画同一圈。
const RING_EPS_KM: f64 = 1e-3;

/// 中国烈度（CSIS）连续值：震级 + 深度 + 震中距（地表 km）。
///
/// 公式来自 RhythmQuake / CEA 衰减：`1.297·M − 4.368·log10(R+15) + 5.363`，
/// 并对断层尺度做简单修正后取平均。
pub fn calc_csis(magnitude: f64, depth_km: f64, surface_km: f64) -> f64 {
    if !magnitude.is_finite() || !surface_km.is_finite() {
        return 0.0;
    }
    if surface_km > 10_000.0 {
        return 0.0;
    }
    let dep = if !depth_km.is_finite() || depth_km < 10.0 {
        10.0
    } else {
        depth_km
    };
    let line_dis = hypocentral_km(dep, surface_km);
    let fault_half = 10f64.powf((magnitude - 3.821) / 1.86);
    let hypo = [
        line_dis - 10.0 - fault_half,
        surface_km - fault_half,
        0.2 * (line_dis - 10.0),
        0.0,
    ]
    .into_iter()
    .fold(f64::NEG_INFINITY, f64::max)
    .max(0.0);

    let a = cea_csis(magnitude, surface_km);
    let b = cea_csis(magnitude, hypo);
    (a + b) / 2.0
}

fn cea_csis(m: f64, dis: f64) -> f64 {
    1.297 * m - 4.368 * ((dis + 15.0).ln() / LN_10) + 5.363
}

/// 震源距（km）：深度 + 地表震中距。
pub fn hypocentral_km(depth_km: f64, surface_km: f64) -> f64 {
    let theta = surface_km / EARTH_RADIUS_KM;
    let a = EARTH_RADIUS_KM - depth_km.max(0.0);
    (a * a + EARTH_RADIUS_KM * EARTH_RADIUS_KM
        - 2.0 * a * EARTH_RADIUS_KM * theta.cos())
        .sqrt()
}

/// JMA 計測震度（仪器震度）估算，对齐 RhythmQuake `calcJmaShindo`（ARV=1）。
pub fn calc_jma_instrumental(
    mj: f64,
    depth_km: f64,
    surface_km: f64,
) -> f64 {
    if !mj.is_finite() || !depth_km.is_finite() || mj < 0.0 || depth_km < 0.0 {
        return -3.0;
    }
    let mw = mj - 0.171;
    let fault_half = 10f64.powf(0.5 * mw - 1.85) / 2.0;
    let line_dis = hypocentral_km(depth_km, surface_km.max(0.0));
    let hypo_dist = (line_dis - fault_half).max(3.0);
    let x = hypo_dist;
    let pgv600 = 10f64.powf(
        0.58 * mw + 0.0038 * depth_km - 1.29
            - ((x + 0.0028 * 10f64.powf(0.5 * mw)).ln() / LN_10)
            - 0.002 * x,
    );
    let pgv = pgv600 * 1.307; // * ARV(1.0)
    if pgv <= 0.0 {
        return -3.0;
    }
    2.68 + 1.72 * (pgv.ln() / LN_10)
}

/// 震中最大震度/烈度：无官方烈度字段时用震级+深度估算（地表距=0）。
pub fn estimate_epicentral(
    kind: IntensityKind,
    magnitude: f64,
    depth_km: f64,
) -> (u8, String) {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return (0, "—".into());
    }
    match kind {
        IntensityKind::JmaShindo => {
            let inst = calc_jma_instrumental(magnitude, depth_km.max(0.0), 0.0);
            if inst <= -3.0 {
                return (0, "—".into());
            }
            let (level, text) = jma_from_instrumental(inst);
            (level, text.into())
        }
        IntensityKind::CnIntensity => {
            let v = calc_csis(magnitude, depth_km, 0.0);
            if v < 0.5 {
                (0, "0".into())
            } else {
                let level = v.round().clamp(1.0, 12.0) as u8;
                (level, cn_intensity_text(level).into())
            }
        }
    }
}

/// 若上游文案为空/`—`，用估算值补齐。
pub fn fill_intensity(
    kind: IntensityKind,
    magnitude: f64,
    depth_km: f64,
    raw: Option<&str>,
) -> (String, u8) {
    let raw = raw.map(str::trim).unwrap_or("");
    // 调查中 / 不明：保留原文，不估算假震度
    if matches!(
        raw,
        "不明"
            | "?"
            | "調査中"
            | "调查中"
            | "なし"
            | "无"
            | "N/A"
            | "n/a"
    ) {
        return (raw.into(), 0);
    }
    if !raw.is_empty() && raw != "—" && raw != "-" && raw != "null" {
        match kind {
            IntensityKind::JmaShindo => {
                let level = crate::jma_shindo_level(raw);
                (raw.into(), level)
            }
            IntensityKind::CnIntensity => {
                let level = crate::cn_intensity_level(raw);
                (cn_intensity_text(level).into(), level)
            }
        }
    } else {
        let (level, text) = estimate_epicentral(kind, magnitude, depth_km);
        (text, level)
    }
}

/// 本地预估烈度/震度（本机位置相对震中）。
pub fn estimate_at_site(
    kind: IntensityKind,
    magnitude: f64,
    depth_km: f64,
    surface_km: f64,
) -> (u8, String) {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return (0, "—".into());
    }
    match kind {
        IntensityKind::JmaShindo => {
            let inst = calc_jma_instrumental(magnitude, depth_km.max(0.0), surface_km.max(0.0));
            if inst <= -3.0 {
                return (0, "—".into());
            }
            let (level, text) = jma_from_instrumental(inst);
            (level, text.into())
        }
        IntensityKind::CnIntensity => {
            let v = calc_csis(magnitude, depth_km, surface_km.max(0.0));
            if v < 0.5 {
                (0, "0".into())
            } else {
                let level = v.round().clamp(1.0, 12.0) as u8;
                (level, cn_intensity_text(level).into())
            }
        }
    }
}

/// 该档位是否参与画环（JMA 1–9 / 中国 1–12）。
fn ring_level_ok(kind: IntensityKind, level: u8) -> bool {
    match kind {
        IntensityKind::JmaShindo => (1..=9).contains(&level),
        IntensityKind::CnIntensity => (1..=12).contains(&level),
    }
}

/// 二分求「离散估档仍 ≥ level」的最大地表距（km）。
///
/// 判定直接走 `estimate_at_site`，与本机倒计时/侧栏同一套衰减→档位映射，
/// 避免连续阈值与 `round`/震度分界不一致。
fn radius_for_level(
    kind: IntensityKind,
    level: u8,
    magnitude: f64,
    depth_km: f64,
) -> Option<f64> {
    if !ring_level_ok(kind, level) {
        return None;
    }
    let at = |r: f64| estimate_at_site(kind, magnitude, depth_km, r).0;
    if at(0.0) < level {
        return None;
    }
    if at(RING_SEARCH_MAX_KM) >= level {
        return Some(RING_SEARCH_MAX_KM);
    }
    let mut lo = 0.0;
    let mut hi = RING_SEARCH_MAX_KM;
    // 56 次二分 → 分辨率约 8000/2^56 ≪ 1 mm，远小于地图像素误差
    for _ in 0..56 {
        let mid = 0.5 * (lo + hi);
        if at(mid) >= level {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if lo < 0.05 {
        None
    } else {
        Some(lo)
    }
}

/// 单档烈度/震度环外半径（地表 km）：该档及以上区域的最远震中距。
/// 震中达不到该档时返回 `None`。
pub fn intensity_radius_km(
    kind: IntensityKind,
    level: u8,
    magnitude: f64,
    depth_km: f64,
) -> Option<f64> {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return None;
    }
    radius_for_level(kind, level, magnitude, depth_km.max(0.0))
}

/// 预警烈度估算环：`(档位, 外半径 km)`，由高到低。
///
/// 半径由衰减正算 + 离散档位反解；仅依赖震级与深度（各向同性圆）。
/// 每一圈表示「估档 ≥ 该档」的外缘，与 `estimate_at_site` 一致。
pub fn intensity_rings(
    kind: IntensityKind,
    magnitude: f64,
    depth_km: f64,
) -> Vec<(u8, f64)> {
    let (max_lv, _) = estimate_epicentral(kind, magnitude, depth_km);
    if max_lv == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut prev_r = f64::INFINITY;
    for level in (1..=max_lv).rev() {
        let Some(r) = intensity_radius_km(kind, level, magnitude, depth_km) else {
            continue;
        };
        if !r.is_finite() || r < 0.05 {
            continue;
        }
        // 仅滤掉与更高档数值重合的圈（浮点噪声），不合并真实不同半径
        if prev_r.is_finite() && (prev_r - r).abs() < RING_EPS_KM {
            continue;
        }
        out.push((level, r));
        prev_r = r;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csis_epicenter_m53_reasonable() {
        let v = calc_csis(5.3, 10.0, 0.0);
        assert!(v > 4.0 && v < 9.0, "got {v}");
    }

    #[test]
    fn jma_small_quake_low_shindo() {
        let (lv, text) = estimate_epicentral(IntensityKind::JmaShindo, 2.9, 10.0);
        assert!(lv <= 3, "lv={lv} text={text}");
    }

    #[test]
    fn cn_rings_monotonic_and_exact_boundary() {
        let m = 6.0;
        let h = 10.0;
        let kind = IntensityKind::CnIntensity;
        let rings = intensity_rings(kind, m, h);
        assert!(!rings.is_empty(), "expected rings");
        for w in rings.windows(2) {
            assert!(w[0].0 > w[1].0);
            assert!(
                w[0].1 < w[1].1,
                "radii should grow outward: L{}={:.6} vs L{}={:.6}",
                w[0].0,
                w[0].1,
                w[1].0,
                w[1].1
            );
        }
        for &(lv, r) in &rings {
            let (on, _) = estimate_at_site(kind, m, h, r);
            assert!(
                on >= lv,
                "on-ring R={r:.6} expected level>={lv} got {on}"
            );
            // 略外侧必须掉档（触顶搜索上限时除外）
            if r < RING_SEARCH_MAX_KM - 1.0 {
                let outside = (r + (r * 1e-6).max(0.02)).min(RING_SEARCH_MAX_KM);
                let (off, _) = estimate_at_site(kind, m, h, outside);
                assert!(
                    off < lv,
                    "outside R={outside:.6} expected level<{lv} got {off} (ring={r:.6})"
                );
            }
        }
    }

    #[test]
    fn jma_rings_exact_boundary() {
        let m = 6.5;
        let h = 10.0;
        let kind = IntensityKind::JmaShindo;
        let rings = intensity_rings(kind, m, h);
        assert!(!rings.is_empty());
        for w in rings.windows(2) {
            assert!(w[0].0 > w[1].0);
            assert!(w[0].1 < w[1].1);
        }
        for &(lv, r) in &rings {
            let (on, _) = estimate_at_site(kind, m, h, r);
            assert!(on >= lv, "on-ring expected>={lv} got {on} R={r}");
            if r < RING_SEARCH_MAX_KM - 1.0 {
                let outside = (r + (r * 1e-6).max(0.02)).min(RING_SEARCH_MAX_KM);
                let (off, _) = estimate_at_site(kind, m, h, outside);
                assert!(
                    off < lv,
                    "outside expected<{lv} got {off} R={outside} ring={r}"
                );
            }
        }
    }

    #[test]
    fn tiny_quake_few_or_no_rings() {
        let rings = intensity_rings(IntensityKind::CnIntensity, 2.0, 10.0);
        assert!(rings.len() <= 3, "got {} rings", rings.len());
    }

    #[test]
    fn radius_matches_estimate_at_site_level() {
        // 任意点：在 L 环内应 ≥L，在下一档环外应 <L
        let kind = IntensityKind::CnIntensity;
        let m = 5.5;
        let h = 12.0;
        let rings = intensity_rings(kind, m, h);
        assert!(rings.len() >= 2);
        let (lv_hi, r_hi) = rings[0];
        let (_lv_lo, r_lo) = *rings.last().unwrap();
        let mid = 0.5 * (r_hi + r_lo);
        let (got, _) = estimate_at_site(kind, m, h, mid);
        assert!(got >= 1);
        // 震中档位应等于最高环档位
        let (epi, _) = estimate_epicentral(kind, m, h);
        assert_eq!(epi, lv_hi);
    }
}
