//! 主界面暗色 / 亮色色板，并同步 egui 全局 visuals。

use egui::{Color32, Context, Visuals};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Dark,
    Light,
}

impl UiTheme {
    pub fn from_config(s: &str) -> Self {
        if s.eq_ignore_ascii_case("light") {
            Self::Light
        } else {
            Self::Dark
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn palette(self) -> ThemePalette {
        match self {
            Self::Dark => ThemePalette::dark(),
            Self::Light => ThemePalette::light(),
        }
    }

    pub fn apply_egui(self, ctx: &Context) {
        let mut v = if self.is_light() {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        let p = self.palette();
        v.panel_fill = p.sidebar_bg;
        v.window_fill = p.panel_solid;
        v.extreme_bg_color = p.map_bg;
        v.override_text_color = Some(p.text);
        v.widgets.noninteractive.fg_stroke.color = p.text;
        v.widgets.inactive.fg_stroke.color = p.text_muted;
        v.widgets.hovered.fg_stroke.color = p.text;
        v.widgets.active.fg_stroke.color = p.text;
        v.selection.bg_fill = p.accent_soft;
        ctx.set_visuals(v);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePalette {
    pub map_bg: Color32,
    pub sidebar_bg: Color32,
    pub panel_glass: Color32,
    pub panel_glass_soft: Color32,
    pub panel_solid: Color32,
    pub panel_border: Color32,
    pub accent_soft: Color32,
    pub chip_bg: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub danger: Color32,
    pub warn: Color32,
    pub ok: Color32,
    pub row_selected: Color32,
    pub tab_idle: Color32,
}

impl ThemePalette {
    /// 暗色监视器：对齐 RhythmQuake desktop-ui-reference（强调色 #82B1FF、玻璃边框）。
    pub fn dark() -> Self {
        Self {
            map_bg: Color32::from_rgb(0x20, 0x21, 0x24),
            sidebar_bg: Color32::from_rgb(0x0A, 0x0A, 0x0A),
            panel_glass: Color32::from_rgba_unmultiplied(0, 0, 0, 120),
            panel_glass_soft: Color32::from_rgba_unmultiplied(0, 0, 0, 90),
            panel_solid: Color32::from_rgb(0x14, 0x14, 0x18),
            panel_border: Color32::from_rgba_unmultiplied(255, 255, 255, 32),
            accent_soft: Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 48),
            chip_bg: Color32::from_rgba_unmultiplied(0, 0, 0, 140),
            text: Color32::from_rgb(0xF2, 0xF4, 0xF7),
            text_muted: Color32::from_rgba_unmultiplied(255, 255, 255, 148),
            text_dim: Color32::from_rgba_unmultiplied(255, 255, 255, 110),
            accent: Color32::from_rgb(0x82, 0xB1, 0xFF),
            danger: Color32::from_rgb(0xEF, 0x44, 0x44),
            warn: Color32::from_rgb(0xFF, 0xC6, 0x6D),
            ok: Color32::from_rgb(0x72, 0xD6, 0xB1),
            row_selected: Color32::from_rgba_unmultiplied(0x82, 0xB1, 0xFF, 40),
            tab_idle: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
        }
    }

    pub fn light() -> Self {
        Self {
            map_bg: Color32::from_rgb(0xC8, 0xD8, 0xE8),
            sidebar_bg: Color32::from_rgb(0xF3, 0xF4, 0xF6),
            panel_glass: Color32::from_rgba_unmultiplied(255, 255, 255, 230),
            panel_glass_soft: Color32::from_rgba_unmultiplied(255, 255, 255, 200),
            panel_solid: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            panel_border: Color32::from_rgba_unmultiplied(0x90, 0x9A, 0xA8, 100),
            accent_soft: Color32::from_rgba_unmultiplied(0x3B, 0x82, 0xF6, 36),
            chip_bg: Color32::from_rgba_unmultiplied(255, 255, 255, 235),
            text: Color32::from_rgb(0x1F, 0x29, 0x37),
            text_muted: Color32::from_rgb(0x4B, 0x55, 0x63),
            text_dim: Color32::from_rgb(0x6B, 0x72, 0x80),
            accent: Color32::from_rgb(0x25, 0x63, 0xEB),
            danger: Color32::from_rgb(0xDC, 0x26, 0x26),
            warn: Color32::from_rgb(0xCA, 0x8A, 0x04),
            ok: Color32::from_rgb(0x15, 0x80, 0x3D),
            row_selected: Color32::from_rgb(0xDB, 0xEA, 0xFE),
            tab_idle: Color32::from_rgb(0xE5, 0xE7, 0xEB),
        }
    }
}
