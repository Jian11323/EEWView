//! 震度/烈度文案 → 档位索引（与 palette 档位对齐）。

use crate::IntensityKind;

/// JMA 震度文案 → 0–9（0 / 1 / 2 / 3 / 4 / 5弱 / 5强 / 6弱 / 6强 / 7）
pub fn jma_shindo_level(text: &str) -> u8 {
    let t = text.trim();
    let t = t
        .replace('弱', "-")
        .replace('強', "+")
        .replace('强', "+");
    match t.as_str() {
        "0" | "０" => 0,
        "1" | "１" => 1,
        "2" | "２" => 2,
        "3" | "３" => 3,
        "4" | "４" => 4,
        "5-" | "5−" => 5,
        "5+" => 6,
        "6-" | "6−" => 7,
        "6+" => 8,
        "7" | "７" => 9,
        _ => {
            // P2PQuake maxScale：10/20/…/70
            if let Ok(n) = t.parse::<i32>() {
                return match n {
                    0 => 0,
                    10 => 1,
                    20 => 2,
                    30 => 3,
                    40 => 4,
                    45 => 5,
                    50 => 6,
                    55 => 7,
                    60 => 8,
                    70 => 9,
                    _ if (0..=9).contains(&n) => n as u8,
                    _ => 0,
                };
            }
            0
        }
    }
}

pub fn jma_shindo_text(level: u8) -> &'static str {
    match level {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5弱",
        6 => "5强",
        7 => "6弱",
        8 => "6强",
        _ => "7",
    }
}

/// 計測震度（仪器震度）→ JMA 震度阶级档位 0–9 与展示文案。
///
/// 分界按气象厅震度階級の解説（0.5 / 1.5 / … / 6.5）。
pub fn jma_from_instrumental(v: f64) -> (u8, &'static str) {
    let level = if v < 0.5 {
        0
    } else if v < 1.5 {
        1
    } else if v < 2.5 {
        2
    } else if v < 3.5 {
        3
    } else if v < 4.5 {
        4
    } else if v < 5.0 {
        5
    } else if v < 5.5 {
        6
    } else if v < 6.0 {
        7
    } else if v < 6.5 {
        8
    } else {
        9
    };
    (level, jma_shindo_text(level))
}

/// 测站列表/地图用展示文案：低于 0.5 显示取整計測值（含 -3～0），否则用震度阶级。
pub fn instrumental_display_text(v: f64) -> String {
    if !v.is_finite() || v < -3.0 {
        "—".into()
    } else if v < 0.5 {
        format!("{}", v.round() as i32)
    } else {
        jma_from_instrumental(v).1.into()
    }
}

/// 中国烈度罗马数字或阿拉伯数字 → 1–12
pub fn cn_intensity_level(text: &str) -> u8 {
    let t = text.trim();
    match t {
        "1" | "Ⅰ" | "I" => 1,
        "2" | "Ⅱ" | "II" => 2,
        "3" | "Ⅲ" | "III" => 3,
        "4" | "Ⅳ" | "IV" => 4,
        "5" | "Ⅴ" | "V" => 5,
        "6" | "Ⅵ" | "VI" => 6,
        "7" | "Ⅶ" | "VII" => 7,
        "8" | "Ⅷ" | "VIII" => 8,
        "9" | "Ⅸ" | "IX" => 9,
        "10" | "Ⅹ" | "X" => 10,
        "11" | "Ⅺ" | "XI" => 11,
        "12" | "Ⅻ" | "XII" => 12,
        _ => {
            if let Ok(f) = t.parse::<f64>() {
                f.round().clamp(1.0, 12.0) as u8
            } else {
                1
            }
        }
    }
}

pub fn cn_intensity_text(level: u8) -> &'static str {
    match level.clamp(1, 12) {
        1 => "Ⅰ",
        2 => "Ⅱ",
        3 => "Ⅲ",
        4 => "Ⅳ",
        5 => "Ⅴ",
        6 => "Ⅵ",
        7 => "Ⅶ",
        8 => "Ⅷ",
        9 => "Ⅸ",
        10 => "Ⅹ",
        11 => "Ⅺ",
        _ => "Ⅻ",
    }
}

/// 按界面「烈度标度」偏好重映射展示（`auto` 保持原文；`jma` / `cn` 强制标度）。
pub fn prefer_intensity_scale(
    kind: IntensityKind,
    level: u8,
    text: &str,
    scale: &str,
) -> (IntensityKind, u8, String) {
    match scale.trim().to_ascii_lowercase().as_str() {
        "jma" => {
            let lv = match kind {
                IntensityKind::JmaShindo => level.min(9),
                IntensityKind::CnIntensity => cn_to_jma_level(level),
            };
            (IntensityKind::JmaShindo, lv, jma_shindo_text(lv).into())
        }
        "cn" => {
            let lv = match kind {
                IntensityKind::CnIntensity => level.clamp(1, 12),
                IntensityKind::JmaShindo => jma_to_cn_level(level),
            };
            (IntensityKind::CnIntensity, lv, cn_intensity_text(lv).into())
        }
        _ => {
            let label = if text.trim().is_empty() {
                match kind {
                    IntensityKind::JmaShindo => jma_shindo_text(level).into(),
                    IntensityKind::CnIntensity => cn_intensity_text(level).into(),
                }
            } else {
                text.to_string()
            };
            (kind, level, label)
        }
    }
}

fn jma_to_cn_level(jma: u8) -> u8 {
    // 粗映射：震度 0–7 阶级 → 烈度约 1–10
    match jma.min(9) {
        0 => 1,
        1 => 2,
        2 => 3,
        3 => 4,
        4 => 5,
        5 => 6,
        6 => 7,
        7 => 8,
        8 => 9,
        _ => 10,
    }
}

fn cn_to_jma_level(cn: u8) -> u8 {
    match cn.clamp(1, 12) {
        1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        9 => 8,
        _ => 9,
    }
}

/// 按机构选择震度/烈度色标
pub fn intensity_kind_for_agency(agency: &str) -> IntensityKind {
    use crate::agency::{agency_family, AgencyFamily};
    match agency_family(agency) {
        AgencyFamily::Jma | AgencyFamily::Cwa | AgencyFamily::Kma => IntensityKind::JmaShindo,
        _ => IntensityKind::CnIntensity,
    }
}
