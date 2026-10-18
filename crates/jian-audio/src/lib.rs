//! 音效引擎：catalog 路由 + rodio 播放 + 倒计时状态机。

mod catalog;
mod countdown;
mod cue;
mod player;
mod router;
mod tts_speak;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub use catalog::SoundCatalog;
pub use countdown::CountdownMachine;
pub use cue::{AudioCue, TsunamiCueKind};
pub use router::Router;
pub use tts_speak::speak_async;

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
        self.play_in_pack(None, cue);
    }

    /// 在指定音效包内播放（如非 JMA 预警用 `eewcn`，JMA 用 `srev`）。
    pub fn play_in_pack(&mut self, pack: Option<&str>, cue: AudioCue) {
        match &cue {
            AudioCue::Countdown(_) => {
                if let Some(path) = self.router.resolve_in_pack(&self.catalog, pack, &cue) {
                    if let Some(p) = &self.player {
                        p.play_speech(&path);
                    }
                }
            }
            AudioCue::CountdownLoop(on) => {
                if *on {
                    if let Some(path) = self.router.resolve_in_pack(&self.catalog, pack, &cue) {
                        if let Some(p) = &self.player {
                            p.set_countdown_loop(Some(&path));
                        }
                    }
                } else if let Some(p) = &self.player {
                    p.set_countdown_loop(None);
                }
            }
            other => {
                if let Some(path) = self.router.resolve_in_pack(&self.catalog, pack, other) {
                    if let Some(p) = &self.player {
                        p.play_oneshot(&path);
                    } else {
                        tracing::debug!(?other, path = %path.display(), "would play (no device)");
                    }
                }
            }
        }
    }

    /// 根据剩余秒驱动地牛倒计时。`start_at` 为开始播报的剩余秒（要石默认 10）。
    pub fn tick_countdown(&mut self, remain_s: Option<i32>, start_at: i32, speech: bool) {
        for cue in self.countdown.tick(remain_s, start_at, speech) {
            self.play(cue);
        }
    }

    /// 启动灌入时静默登记已见报，避免稍后把历史首报当新 issue 再播。
    pub fn seed_eew_seen(&mut self, agency: &str, event_id: &str, serial: u32) {
        let key = format!("{agency}|{event_id}");
        let prev = self.eew_seen.get(&key).copied().unwrap_or(0);
        if serial >= prev {
            self.eew_seen.insert(key, serial);
        }
    }

    /// 预警事件音：`pack` 为 `srev`（JMA）或 `eewcn`（其余机构）。
    /// SREV 另附 caution / warn；eewcn 仅 issue / update / final。
    pub fn on_eew(
        &mut self,
        agency: &str,
        event_id: &str,
        serial: u32,
        is_final: bool,
        is_cancel: bool,
        is_warn: bool,
        play_caution: bool,
        pack: &str,
    ) {
        let key = format!("{agency}|{event_id}");
        let pack = Some(pack);
        if is_cancel {
            self.play_in_pack(pack, AudioCue::Cancel);
            self.eew_seen.insert(key, serial);
            return;
        }
        let prev = self.eew_seen.get(&key).copied();
        let advanced = match prev {
            None => {
                self.play_in_pack(pack, AudioCue::Issue);
                true
            }
            Some(p) if serial > p => {
                if is_final {
                    self.play_in_pack(pack, AudioCue::Final);
                } else {
                    self.play_in_pack(pack, AudioCue::Update);
                }
                true
            }
            _ => false, // 同 serial 或更旧
        };
        if !advanced {
            return;
        }
        // SREV / 要石：警报播 warn（并吞掉 caution）；否则震度≥3 播 caution
        if pack == Some("srev") {
            if is_warn {
                if !self.warn_played.contains(&key) {
                    self.play_in_pack(pack, AudioCue::Warn);
                    self.warn_played.insert(key.clone());
                    self.caution_played.insert(key.clone());
                }
            } else if play_caution && !self.caution_played.contains(&key) {
                self.play_in_pack(pack, AudioCue::Caution);
                self.caution_played.insert(key.clone());
            }
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
