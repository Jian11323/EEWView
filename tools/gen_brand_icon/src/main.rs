//! 从 icon/brand/logo.svg 生成窗口/exe 用 PNG 与多尺寸 ICO。
use image::codecs::ico::{IcoEncoder, IcoFrame};
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("tools/gen_brand_icon -> repo root")
        .to_path_buf()
}

fn rasterize(svg: &[u8], max_px: u32) -> (u32, u32, Vec<u8>) {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg, &opt).expect("parse logo.svg");
    let size = tree.size();
    let sw = size.width().max(1.0);
    let sh = size.height().max(1.0);
    let scale = max_px as f32 / sw.max(sh);
    let w = (sw * scale).round().max(1.0) as u32;
    let h = (sh * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).expect("pixmap");
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia 为预乘；image/ICO 与 egui IconData 要非预乘
    let mut rgba = pixmap.data().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as f32;
        if a > 0.0 && a < 255.0 {
            let inv = 255.0 / a;
            px[0] = (px[0] as f32 * inv).round().min(255.0) as u8;
            px[1] = (px[1] as f32 * inv).round().min(255.0) as u8;
            px[2] = (px[2] as f32 * inv).round().min(255.0) as u8;
        }
    }
    (w, h, rgba)
}

fn main() {
    let root = repo_root();
    let brand = root.join("icon/brand");
    let svg = fs::read(brand.join("logo.svg")).expect("read logo.svg");

    let (w, h, rgba) = rasterize(&svg, 256);
    let png_path = brand.join("app.png");
    image::save_buffer(&png_path, &rgba, w, h, image::ColorType::Rgba8).expect("write app.png");
    println!("wrote {} ({}x{})", png_path.display(), w, h);

    let mut frames = Vec::new();
    for size in [16u32, 32, 48, 64, 128, 256] {
        let (iw, ih, data) = rasterize(&svg, size);
        frames.push(
            IcoFrame::as_png(&data, iw, ih, image::ExtendedColorType::Rgba8).expect("ico frame"),
        );
    }
    let ico_path = brand.join("app.ico");
    let file = fs::File::create(&ico_path).expect("create app.ico");
    IcoEncoder::new(file)
        .encode_images(&frames)
        .expect("write app.ico");
    println!("wrote {} ({} sizes)", ico_path.display(), frames.len());
}
