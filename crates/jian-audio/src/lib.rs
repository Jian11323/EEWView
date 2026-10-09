//! 音效引擎：catalog 路由 + rodio 播放 + 倒计时状态机。

mod catalog;
mod countdown;
mod cue;
mod player;
mod router;

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub use catalog::SoundCatalog;
pub use countdown::CountdownMachine;
pub use cue::AudioCue;
pub use router::Router;

use player::Player;

pub struct AudioEngine {
    catalog: SoundCatalog,
    router: Router,
    player: Option<Player>,
    countdown: CountdownMachine,
    /// 已播过的 EEW 键 → 上次 serial（避免同报重复）
    eew_seen: HashMap<String, u32>,
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

    /// 新/更新 EEW 时调用：首报 Issue，serial 升高 Update，可附带震度档。
    pub fn on_eew(
        &mut self,
        agency: &str,
        event_id: &str,
        serial: u32,
        is_final: bool,
        is_cancel: bool,
        shindo_level: Option<u8>,
    ) {
        let key = format!("{agency}|{event_id}");
        if is_cancel {
            self.play(AudioCue::Cancel);
            self.eew_seen.insert(key, serial);
            return;
        }
        let prev = self.eew_seen.get(&key).copied();
        match prev {
            None => {
                self.play(AudioCue::Issue);
                if let Some(n) = shindo_level {
                    self.play(AudioCue::Shindo(n));
                }
            }
            Some(p) if serial > p => {
                if is_final {
                    self.play(AudioCue::Final);
                } else {
                    self.play(AudioCue::Update);
                }
            }
            _ => return, // 同 serial 或更旧
        }
        self.eew_seen.insert(key, serial);
    }

    pub fn set_mute(&mut self, mute: bool) {
        if let Some(p) = &mut self.player {
            p.set_mute(mute);
        }
    }
}
