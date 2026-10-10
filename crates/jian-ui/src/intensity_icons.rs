//! 烈度/震度图标：`assets/intensity/csis` 与 `assets/intensity/shindo`（SVG）。
//!
//! JMA：level 0–9 → 0,1,2,3,4,5弱,5强,6弱,6强,7。

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use jian_core::IntensityKind;
use std::collections::HashMap;

macro_rules! svg {
    ($rel:expr) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/intensity/",
            $rel
        ))
    };
}

const RASTER_PX: u32 = 128;

const SHINDO_SVG: [(&[u8], u8); 10] = [
    (svg!("shindo/Shindo 0.svg"), 0),
    (svg!("shindo/Shindo 1.svg"), 1),
    (svg!("shindo/Shindo 2.svg"), 2),
    (svg!("shindo/Shindo 3.svg"), 3),
    (svg!("shindo/Shindo 4.svg"), 4),
    (svg!("shindo/Shindo 5-.svg"), 5),
    (svg!("shindo/Shindo 5+.svg"), 6),
    (svg!("shindo/Shindo 6-.svg"), 7),
    (svg!("shindo/Shindo 6+.svg"), 8),
    (svg!("shindo/Shindo 7.svg"), 9),
];

const CSIS_SVG: [(&[u8], u8); 12] = [
    (svg!("csis/CSIS 1.svg"), 1),
    (svg!("csis/CSIS 2.svg"), 2),
    (svg!("csis/CSIS 3.svg"), 3),
    (svg!("csis/CSIS 4.svg"), 4),
    (svg!("csis/CSIS 5.svg"), 5),
    (svg!("csis/CSIS 6.svg"), 6),
    (svg!("csis/CSIS 7.svg"), 7),
    (svg!("csis/CSIS 8.svg"), 8),
    (svg!("csis/CSIS 9.svg"), 9),
    (svg!("csis/CSIS 10.svg"), 10),
    (svg!("csis/CSIS 11.svg"), 11),
    (svg!("csis/CSIS 12.svg"), 12),
];

#[derive(Default)]
pub struct IntensityIcons {
    jma: HashMap<u8, TextureHandle>,
    csis: HashMap<u8, TextureHandle>,
    tried: bool,
}

impl IntensityIcons {
    pub fn ensure(&mut self, ctx: &Context) {
        if self.tried {
            return;
        }
        self.tried = true;
        for (bytes, level) in SHINDO_SVG {
            if let Some(tex) = load_svg(ctx, &format!("eewview/shindo/{level}"), bytes) {
                self.jma.insert(level, tex);
            }
        }
        for (bytes, level) in CSIS_SVG {
            if let Some(tex) = load_svg(ctx, &format!("eewview/csis/{level}"), bytes) {
                self.csis.insert(level, tex);
            }
        }
        if self.jma.len() < 10 || self.csis.len() < 12 {
            tracing::warn!("部分烈度/震度 SVG 未能加载，将回退为色块文字");
        }
    }

    pub fn get(&self, kind: IntensityKind, level: u8) -> Option<&TextureHandle> {
        match kind {
            IntensityKind::JmaShindo => self.jma.get(&level.min(9)),
            IntensityKind::CnIntensity => {
                let lv = level.clamp(1, 12);
                self.csis.get(&lv)
            }
        }
    }
}

fn load_svg(ctx: &Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    let image = rasterize_svg(bytes, RASTER_PX)?;
    Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
}

fn rasterize_svg(bytes: &[u8], max_px: u32) -> Option<ColorImage> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).ok()?;
    let size = tree.size();
    let sw = size.width().max(1.0);
    let sh = size.height().max(1.0);
    let scale = max_px as f32 / sw.max(sh);
    let w = (sw * scale).round().max(1.0) as u32;
    let h = (sh * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(ColorImage::from_rgba_premultiplied(
        [w as usize, h as usize],
        pixmap.data(),
    ))
}
