//! 产品显示名与品牌图标（窗口 / 关于页）。

use egui::{ColorImage, Context, IconData, TextureHandle, TextureOptions};

/// 面向用户的软件名称。
pub const APP_DISPLAY_NAME: &str = "EEW View";

const APP_PNG: &[u8] = include_bytes!("../../../icon/brand/app.png");

/// 主窗口 / 设置窗的任务栏与标题栏图标。
pub fn app_icon_data() -> Option<IconData> {
    let img = image::load_from_memory(APP_PNG).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Some(IconData {
        rgba: img.into_raw(),
        width: w,
        height: h,
    })
}

/// 关于页等 UI 用的 logo 贴图（缓存于 egui 纹理）。
pub fn logo_texture(ctx: &Context) -> Option<TextureHandle> {
    let img = image::load_from_memory(APP_PNG).ok()?.into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("eewview/brand_logo", color, TextureOptions::LINEAR))
}
