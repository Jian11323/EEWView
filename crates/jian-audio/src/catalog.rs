//! 解析 `assets/sound/catalog.json`。

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct SoundCatalog {
    pub version: u32,
    pub routing: HashMap<String, String>,
    pub packs: HashMap<String, PackDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackDef {
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default)]
    pub events: HashMap<String, String>,
}

impl SoundCatalog {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref())
            .with_context(|| format!("read {}", path.as_ref().display()))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// 路由键 → 包名（如 `default_eew` → `srev`）
    pub fn pack_for_route(&self, route: &str) -> Option<&str> {
        self.routing.get(route).map(|s| s.as_str())
    }

    /// 包内事件键 → 相对 catalog 根的路径
    pub fn event_rel_path(&self, pack: &str, event: &str) -> Option<PathBuf> {
        let p = self.packs.get(pack)?;
        p.events.get(event).map(PathBuf::from)
    }
}
