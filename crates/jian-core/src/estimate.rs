//! 震中最大震度/烈度估算（对齐 RhythmQuake `IntensityCalculator`）。

use crate::{cn_intensity_text, jma_from_instrumental, IntensityKind};
use jian_util::EARTH_RADIUS_KM;
use std::f64::consts::LN_10;

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
}
