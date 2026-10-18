//! 解析 `media/sound/catalog.json`。

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn catalog_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../media/sound/catalog.json")
    }

    fn sound_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../media/sound")
    }

    #[test]
    fn loads_workspace_catalog() {
        let cat = SoundCatalog::load(catalog_path()).expect("catalog");
        assert!(cat.version >= 2);
        assert_eq!(cat.pack_for_route("default_eew"), Some("eewcn"));
        assert_eq!(cat.pack_for_route("jma_eew"), Some("srev"));
        assert_eq!(cat.pack_for_route("record"), Some("eewcn"));
        assert_eq!(cat.pack_for_route("ui"), Some("ui"));
        let issue = cat.event_rel_path("eewcn", "issue").expect("eewcn/issue");
        assert!(sound_root().join(issue).is_file());
        let record = cat.event_rel_path("eewcn", "record").expect("eewcn/record");
        assert!(sound_root().join(record).is_file());
        let notify = cat.event_rel_path("ui", "notify").expect("ui/notify");
        assert!(sound_root().join(notify).is_file());
    }
}
