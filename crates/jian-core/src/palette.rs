//! 官方震度/烈度色。
//!
//! 震度：气象厅《配色に関する設定指針》表２－２（令和2年7月）。
//! 烈度：按 GB/T 38226—2019 附录摘录（见 assets/palette/cn_intensity.toml）。

use crate::IntensityKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_egui(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

/// JMA 震度 0–9：0,1,2,3,4,5弱,5强,6弱,6强,7
pub fn jma_shindo_rgb(level: u8) -> Rgb {
    match level.min(9) {
        0 => Rgb::new(64, 64, 64),
        1 => Rgb::new(242, 242, 255),
        2 => Rgb::new(0, 170, 255),
        3 => Rgb::new(0, 65, 255),
        4 => Rgb::new(250, 230, 150),
        5 => Rgb::new(255, 230, 0),   // 5弱
        6 => Rgb::new(255, 153, 0),   // 5强
        7 => Rgb::new(255, 40, 0),    // 6弱
        8 => Rgb::new(165, 0, 33),    // 6强
        _ => Rgb::new(180, 0, 104),   // 7
    }
}

/// 中国烈度 1–12（RGB 以 palette 文件与国标附录为准）
pub fn cn_intensity_rgb(level: u8) -> Rgb {
    match level.clamp(1, 12) {
        1 => Rgb::new(200, 220, 255),
        2 => Rgb::new(160, 200, 255),
        3 => Rgb::new(100, 180, 255),
        4 => Rgb::new(80, 200, 120),
        5 => Rgb::new(255, 255, 100),
        6 => Rgb::new(255, 200, 50),
        7 => Rgb::new(255, 140, 0),
        8 => Rgb::new(255, 60, 0),
        9 => Rgb::new(200, 0, 40),
        10 => Rgb::new(140, 0, 80),
        11 => Rgb::new(100, 0, 100),
        _ => Rgb::new(60, 0, 60),
    }
}

pub fn color_for(kind: IntensityKind, level: u8) -> Rgb {
    match kind {
        IntensityKind::JmaShindo => jma_shindo_rgb(level),
        IntensityKind::CnIntensity => cn_intensity_rgb(level),
    }
}

/// 要石 / NIED 計測震度色带（level 0–20，对应约 -3～7）。
/// `#0003cf` 为 -3 深蓝。
const INSTRUMENTAL_BAND: [Rgb; 21] = [
    Rgb::new(0x00, 0x03, 0xcf),
    Rgb::new(0x00, 0x14, 0xda),
    Rgb::new(0x00, 0x37, 0xf0),
    Rgb::new(0x00, 0x6c, 0xdc),
    Rgb::new(0x00, 0xb3, 0xa2),
    Rgb::new(0x12, 0xdc, 0x72),
    Rgb::new(0x31, 0xf0, 0x49),
    Rgb::new(0x64, 0xfb, 0x2a),
    Rgb::new(0x9d, 0xfe, 0x17),
    Rgb::new(0xcc, 0xff, 0x09),
    Rgb::new(0xeb, 0xff, 0x03),
    Rgb::new(0xff, 0xf5, 0x00),
    Rgb::new(0xff, 0xe5, 0x00),
    Rgb::new(0xff, 0xca, 0x00),
    Rgb::new(0xff, 0xa6, 0x00),
    Rgb::new(0xff, 0x7e, 0x00),
    Rgb::new(0xff, 0x59, 0x00),
    Rgb::new(0xfd, 0x35, 0x00),
    Rgb::new(0xf8, 0x11, 0x00),
    Rgb::new(0xe5, 0x00, 0x00),
    Rgb::new(0xbd, 0x00, 0x00),
];

/// 計測震度 → 色带档位 0–20（对齐要石 `getLevelFromInstShindo`）。
/// `< -3` 返回 `None`（缺测）。
pub fn instrumental_band_level(v: f64) -> Option<u8> {
    if !v.is_finite() || v < -3.0 {
        None
    } else if (v + 3.0).abs() < 1e-9 {
        Some(0)
    } else if v >= 6.5 {
        Some(20)
    } else {
        Some((v * 2.0 + 7.0).floor().clamp(0.0, 20.0) as u8)
    }
}

/// 計測震度上色；缺测时返回中性灰。
pub fn instrumental_rgb(v: f64) -> Rgb {
    match instrumental_band_level(v) {
        Some(lv) => INSTRUMENTAL_BAND[lv as usize],
        None => Rgb::new(0xcf, 0xcf, 0xcf),
    }
}

/// 图例等离散取样：给定色带档位取色。
pub fn instrumental_band_rgb(level: u8) -> Rgb {
    INSTRUMENTAL_BAND[level.min(20) as usize]
}
