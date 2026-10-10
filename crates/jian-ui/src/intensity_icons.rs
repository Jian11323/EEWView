//! CSIS 烈度（`assets/CSIS`）与 JMA 震度（`assets/zd`）图标。
//!
//! JMA：5弱→`5R`，5强→`5Q`；6弱→`6R`，6强→`6Q`。

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use jian_core::IntensityKind;
use std::collections::HashMap;

macro_rules! png {
    ($rel:expr) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/", $rel))
    };
}

const CSIS_PNG: [&[u8]; 11] = [
    png!("CSIS/1.png"),
    png!("CSIS/2.png"),
    png!("CSIS/3.png"),
    png!("CSIS/4.png"),
    png!("CSIS/5.png"),
    png!("CSIS/6.png"),
    png!("CSIS/7.png"),
    png!("CSIS/8.png"),
    png!("CSIS/9.png"),
    png!("CSIS/10.png"),
    png!("CSIS/11.png"),
];

const JMA_PNG: [(&[u8], &str); 10] = [
    (png!("zd/0.png"), "0"),
    (png!("zd/1.png"), "1"),
    (png!("zd/2.png"), "2"),
    (png!("zd/3.png"), "3"),
    (png!("zd/4.png"), "4"),
    (png!("zd/5R.png"), "5R"),
    (png!("zd/5Q.png"), "5Q"),
    (png!("zd/6R.png"), "6R"),
    (png!("zd/6Q.png"), "6Q"),
    (png!("zd/7.png"), "7"),
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
        for (i, (bytes, name)) in JMA_PNG.iter().enumerate() {
            if let Some(tex) = load_png(ctx, &format!("eewview/zd/{name}"), bytes) {
                self.jma.insert(i as u8, tex);
            }
        }
        for (i, bytes) in CSIS_PNG.iter().enumerate() {
            let level = (i + 1) as u8;
            if let Some(tex) = load_png(ctx, &format!("eewview/csis/{level}"), bytes) {
                self.csis.insert(level, tex);
            }
        }
        if self.jma.len() < 10 || self.csis.len() < 11 {
            tracing::warn!("部分烈度/震度图标未能加载，将回退为色块文字");
        }
    }

    pub fn get(&self, kind: IntensityKind, level: u8) -> Option<&TextureHandle> {
        match kind {
            IntensityKind::JmaShindo => self.jma.get(&level.min(9)),
            IntensityKind::CnIntensity => {
                let lv = level.clamp(1, 12);
                self.csis.get(&lv.min(11))
            }
        }
    }
}

fn load_png(ctx: &Context, name: &str, bytes: &[u8]) -> Option<TextureHandle> {
    let img = image::load_from_memory(bytes).ok()?.into_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture(name, color, TextureOptions::LINEAR))
}
