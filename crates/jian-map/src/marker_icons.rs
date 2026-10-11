//! 地图标记 SVG：预警 cross / 速报 red / 本机蓝房子；测站震度 shindo_inst。

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;

const CROSS_SVG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/markers/cross.svg"
));
const RED_SVG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/markers/red.svg"
));
const GREEN_SVG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/markers/green.svg"
));

macro_rules! shindo_inst {
    ($name:expr) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/intensity/shindo_inst/",
            $name
        ))
    };
}

const SHINDO_INST_SVG: [(&[u8], u8); 10] = [
    (shindo_inst!("Shindo 0.svg"), 0),
    (shindo_inst!("Shindo 1.svg"), 1),
    (shindo_inst!("Shindo 2.svg"), 2),
    (shindo_inst!("Shindo 3.svg"), 3),
    (shindo_inst!("Shindo 4.svg"), 4),
    (shindo_inst!("Shindo 5-.svg"), 5),
    (shindo_inst!("Shindo 5+.svg"), 6),
    (shindo_inst!("Shindo 6-.svg"), 7),
    (shindo_inst!("Shindo 6+.svg"), 8),
    (shindo_inst!("Shindo 7.svg"), 9),
];

/// 栅格化尺寸（屏幕绘制时再缩放）
const RASTER_PX: u32 = 128;

#[derive(Default)]
pub struct MarkerIcons {
    cross: Option<TextureHandle>,
    red: Option<TextureHandle>,
    green: Option<TextureHandle>,
    /// JMA 仪器震度图标（level 0–9）
    shindo_inst: HashMap<u8, TextureHandle>,
    tried: bool,
}

impl MarkerIcons {
    /// 首次绘制时上传纹理；失败则保持 None，由调用方几何回退。
    pub fn ensure(&mut self, ctx: &Context) {
        if self.tried {
            return;
        }
        self.tried = true;
        self.cross = load_svg_texture(ctx, "eewview/marker_cross", CROSS_SVG);
        self.red = load_svg_texture(ctx, "eewview/marker_red", RED_SVG);
        self.green = load_svg_texture(ctx, "eewview/marker_green", GREEN_SVG);
        for (bytes, level) in SHINDO_INST_SVG {
            if let Some(tex) =
                load_svg_texture(ctx, &format!("eewview/map_shindo_inst/{level}"), bytes)
            {
                self.shindo_inst.insert(level, tex);
            }
        }
        if self.cross.is_none() || self.red.is_none() || self.green.is_none() {
            tracing::warn!("部分地图标记 SVG 未能栅格化，将使用几何回退");
        }
        if self.shindo_inst.len() < 10 {
            tracing::warn!("测站 shindo_inst 未全部加载，有感点将回退色点");
        }
    }

    pub fn cross(&self) -> Option<&TextureHandle> {
        self.cross.as_ref()
    }

    pub fn red(&self) -> Option<&TextureHandle> {
        self.red.as_ref()
    }

    pub fn green(&self) -> Option<&TextureHandle> {
        self.green.as_ref()
    }

    /// 测站实际震度（>=1）用仪器震度图标。
    pub fn shindo_inst(&self, level: u8) -> Option<&TextureHandle> {
        self.shindo_inst.get(&level.min(9))
    }
}

fn load_svg_texture(ctx: &Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
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
