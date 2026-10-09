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
