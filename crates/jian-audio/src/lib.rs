//! 音效引擎：catalog 路由 + rodio 播放 + 倒计时状态机。

mod catalog;
mod countdown;
mod cue;
mod player;
mod router;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub use catalog::SoundCatalog;
pub use countdown::CountdownMachine;
pub use cue::{AudioCue, TsunamiCueKind};
pub use router::Router;

use player::Player;

pub struct AudioEngine {
    catalog: SoundCatalog,
    router: Router,
    player: Option<Player>,
    countdown: CountdownMachine,
    /// 已播过的 EEW 键 → 上次 serial（避免同报重复）
    eew_seen: HashMap<String, u32>,
    /// 本事件已播 caution / warn（对齐要石：各最多一次）
    caution_played: HashSet<String>,
    warn_played: HashSet<String>,
}

impl AudioEngine {
    pub fn from_catalog(
        catalog_path: impl AsRef<Path>,
        sound_root: impl Into<PathBuf>,
        event_pack: &str,
        countdown_pack: &str,
        master_volume: f32,
        countdown_volume: f32,
        mute: bool,
    ) -> Result<Self> {
        let catalog_path = catalog_path.as_ref();
        let catalog = SoundCatalog::load(catalog_path)
            .with_context(|| format!("load catalog {}", catalog_path.display()))?;
        info!(
            version = catalog.version,
            routing = ?catalog.routing,
            "audio catalog loaded"
        );
        let router = Router::new(sound_root, event_pack, countdown_pack);
        let player = match Player::try_open(master_volume, countdown_volume, mute) {
            Ok(p) => Some(p),
            Err(e) => {
                warn!("audio device unavailable, mute mode: {e:#}");
                None
            }
        };
        Ok(Self {
            catalog,
            router,
            player,
            countdown: CountdownMachine::default(),
            eew_seen: HashMap::new(),
            caution_played: HashSet::new(),
            warn_played: HashSet::new(),
        })
    }

    pub fn play(&mut self, cue: AudioCue) {
        match &cue {
            AudioCue::CountdownLoop(on) => {
                if *on {
                    if let Some(path) = self.router.resolve(&self.catalog, &cue) {
                        if let Some(p) = &self.player {
                            p.set_countdown_loop(Some(&path));
                        }
                    }
                } else if let Some(p) = &self.player {
                    p.set_countdown_loop(None);
                }
            }
            other => {
                if let Some(path) = self.router.resolve(&self.catalog, other) {
                    if let Some(p) = &self.player {
                        p.play_oneshot(&path);
                    } else {
                        tracing::debug!(?other, path = %path.display(), "would play (no device)");
                    }
                }
            }
        }
    }

    /// 根据剩余秒驱动倒计时播报（有本机位置时才应传入 Some）。
    pub fn tick_countdown(&mut self, remain_s: Option<i32>) {
        for cue in self.countdown.tick(remain_s) {
            self.play(cue);
        }
    }

    /// JMA EEW（对齐要石）：issue / update / final / cancel，另附 caution / warn。
    /// `is_warn`：警报；`play_caution`：震度≥3 且非警报附加 caution。
    pub fn on_eew(
        &mut self,
        agency: &str,
        event_id: &str,
        serial: u32,
        is_final: bool,
        is_cancel: bool,
        is_warn: bool,
        play_caution: bool,
    ) {
        let key = format!("{agency}|{event_id}");
        if is_cancel {
            self.play(AudioCue::Cancel);
            self.eew_seen.insert(key, serial);
            return;
        }
        let prev = self.eew_seen.get(&key).copied();
        let advanced = match prev {
            None => {
                self.play(AudioCue::Issue);
                true
            }
            Some(p) if serial > p => {
                if is_final {
                    self.play(AudioCue::Final);
                } else {
                    self.play(AudioCue::Update);
                }
                true
            }
            _ => false, // 同 serial 或更旧
        };
        if !advanced {
            return;
        }
        // 要石：警报播 warn（并吞掉 caution）；否则震度≥3 播 caution；各事件最多一次
        if is_warn {
            if !self.warn_played.contains(&key) {
                self.play(AudioCue::Warn);
                self.warn_played.insert(key.clone());
                self.caution_played.insert(key.clone());
            }
        } else if play_caution && !self.caution_played.contains(&key) {
            self.play(AudioCue::Caution);
            self.caution_played.insert(key.clone());
        }
        self.eew_seen.insert(key, serial);
    }

    pub fn set_mute(&mut self, mute: bool) {
        if let Some(p) = &mut self.player {
            p.set_mute(mute);
        }
    }

    pub fn set_volumes(&mut self, master: f32, countdown_volume: f32) {
        if let Some(p) = &mut self.player {
            p.set_volumes(master, countdown_volume);
        }
    }

    pub fn set_packs(&mut self, event_pack: &str, countdown_pack: &str) {
        self.router.set_packs(event_pack, countdown_pack);
    }
}
