//! Cue + 事件包配置 → 文件路径。

use std::path::{Path, PathBuf};

use crate::catalog::SoundCatalog;
use crate::cue::AudioCue;

pub struct Router {
    sound_root: PathBuf,
    /// 配置覆盖：事件包名（默认走 routing.default_eew）
    event_pack: String,
    countdown_pack: String,
}

impl Router {
    pub fn new(sound_root: impl Into<PathBuf>, event_pack: &str, countdown_pack: &str) -> Self {
        Self {
            sound_root: sound_root.into(),
            event_pack: event_pack.to_string(),
            countdown_pack: countdown_pack.to_string(),
        }
    }

    pub fn set_packs(&mut self, event_pack: &str, countdown_pack: &str) {
        self.event_pack = event_pack.to_string();
        self.countdown_pack = countdown_pack.to_string();
    }

    pub fn sound_root(&self) -> &Path {
        &self.sound_root
    }

    pub fn resolve(&self, catalog: &SoundCatalog, cue: &AudioCue) -> Option<PathBuf> {
        let (pack, event_key) = match cue {
            AudioCue::Issue => (self.event_pack_name(catalog).to_string(), "issue".into()),
            AudioCue::Update => (self.event_pack_name(catalog).to_string(), "update".into()),
            AudioCue::Final => (self.event_pack_name(catalog).to_string(), "final".into()),
            AudioCue::Cancel => (self.event_pack_name(catalog).to_string(), "cancel".into()),
            AudioCue::Shindo(n) => (
                self.event_pack_name(catalog).to_string(),
                format!("shindo_{}", (*n).min(7)),
            ),
            AudioCue::Countdown(sec) => (self.countdown_pack.clone(), format!("{sec}s")),
            AudioCue::CountdownLoop(_) => (self.countdown_pack.clone(), "countdown_loop".into()),
            AudioCue::UiConnectOk => ("ui".into(), "connect_ok".into()),
            AudioCue::UiConnectFail => ("ui".into(), "connect_fail".into()),
            AudioCue::UiNotify => ("ui".into(), "notify".into()),
        };

        let rel = catalog.event_rel_path(&pack, &event_key)?;
        let full = self.sound_root.join(&rel);
        if full.is_file() {
            return Some(full);
        }
        // 回退原创 eew 包（无 SREV 文件时）
        if pack != "eew" {
            if let Some(rel2) = catalog.event_rel_path("eew", &event_key) {
                let f2 = self.sound_root.join(rel2);
                if f2.is_file() {
                    return Some(f2);
                }
            }
        }
        tracing::warn!(path = %full.display(), "audio file missing");
        None
    }

    fn event_pack_name<'a>(&'a self, catalog: &'a SoundCatalog) -> &'a str {
        if catalog.packs.contains_key(&self.event_pack) {
            &self.event_pack
        } else {
            catalog.pack_for_route("default_eew").unwrap_or("srev")
        }
    }
}
