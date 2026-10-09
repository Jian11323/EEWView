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

/// 按机构粗分色标（骨架：含 jma → JMA；cea/cenc 等 → CN）
pub fn intensity_kind_for_agency(agency: &str) -> IntensityKind {
    let a = agency.to_ascii_lowercase();
    if a.contains("jma") || a.contains("wolfx") || a.contains("p2p") {
        IntensityKind::JmaShindo
    } else if a.contains("cea")
        || a.contains("cenc")
        || a.contains("cwa")
        || a.contains("ningxia")
        || a.contains("yunnan")
    {
        IntensityKind::CnIntensity
    } else {
        IntensityKind::JmaShindo
    }
}
