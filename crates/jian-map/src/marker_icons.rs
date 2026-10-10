//! 地图标记 SVG（eewcn 授权）：预警 cross / 速报 red / 本机 green。

use egui::{ColorImage, Context, TextureHandle, TextureOptions};

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
/// 栅格化尺寸（屏幕绘制时再缩放）
const RASTER_PX: u32 = 128;

#[derive(Default)]
pub struct MarkerIcons {
    cross: Option<TextureHandle>,
    red: Option<TextureHandle>,
    green: Option<TextureHandle>,
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
        if self.cross.is_none() || self.red.is_none() || self.green.is_none() {
            tracing::warn!("部分地图标记 SVG 未能栅格化，将使用几何回退");
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
