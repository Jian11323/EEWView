//! 配置加载。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub map: MapConfig,
    pub audio: AudioConfig,
    pub travel: TravelConfig,
    pub local: LocalConfig,
    pub sources: SourcesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub theme: String,
    pub show_nied_clock: bool,
    pub sidebar_default_tab: String,
    pub intensity_scale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapConfig {
    pub tile_base: String,
    pub tile_source: String,
    pub default_lon: f64,
    pub default_lat: f64,
    pub default_zoom: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub event_pack: String,
    pub countdown_pack: String,
    pub master_volume: f32,
    pub countdown_volume: f32,
    pub mute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelConfig {
    pub primary: String,
    pub fallback: String,
    pub linear_last_resort: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcesConfig {
    pub jian: JianSourceConfig,
    pub wolfx: WolfxSourceConfig,
    pub p2pquake: P2pSourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JianSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 建议用 `/all` 单连接，避免占满并发额度
    #[serde(default = "default_jian_ws")]
    pub ws_url: String,
    /// 访问令牌 `at_…`；优先环境变量 `JIAN_ACCESS_TOKEN`，勿写入仓库
    #[serde(default)]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WolfxSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_wolfx_ws")]
    pub ws_urls: Vec<String>,
    /// 冷启动速报列表（HTTP JSON）
    #[serde(default = "default_wolfx_eqlist")]
    pub http_eqlist: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_p2p_http")]
    pub http_history: String,
    #[serde(default = "default_p2p_ws")]
    pub ws_url: String,
    #[serde(default = "default_p2p_poll")]
    pub poll_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_jian_ws() -> String {
    "wss://api.sismotide.top/all".into()
}
fn default_wolfx_ws() -> Vec<String> {
    vec!["wss://ws-api.wolfx.jp/jma_eew".into()]
}
fn default_wolfx_eqlist() -> String {
    "https://api.wolfx.jp/cenc_eqlist.json".into()
}
fn default_p2p_http() -> String {
    "https://api.p2pquake.net/v2/history".into()
}
fn default_p2p_ws() -> String {
    "wss://api.p2pquake.net/v2/ws".into()
}
fn default_p2p_poll() -> u64 {
    30
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig {
                theme: "dark".into(),
                show_nied_clock: false,
                sidebar_default_tab: "eew".into(),
                intensity_scale: "auto".into(),
            },
            map: MapConfig {
                tile_base: "https://tilemap.sismotide.top".into(),
                tile_source: "arcwob".into(),
                default_lon: 120.0,
                default_lat: 30.0,
                default_zoom: 5.0,
            },
            audio: AudioConfig {
                event_pack: "srev".into(),
                countdown_pack: "countdown".into(),
                master_volume: 0.8,
                countdown_volume: 1.0,
                mute: false,
            },
            travel: TravelConfig {
                primary: "jma2001".into(),
                fallback: "ak135".into(),
                linear_last_resort: true,
            },
            local: LocalConfig {
                latitude: None,
                longitude: None,
            },
            sources: SourcesConfig {
                jian: JianSourceConfig {
                    enabled: true,
                    ws_url: default_jian_ws(),
                    access_token: None,
                },
                wolfx: WolfxSourceConfig {
                    enabled: true,
                    ws_urls: default_wolfx_ws(),
                    http_eqlist: default_wolfx_eqlist(),
                },
                p2pquake: P2pSourceConfig {
                    enabled: true,
                    http_history: default_p2p_http(),
                    ws_url: default_p2p_ws(),
                    poll_secs: default_p2p_poll(),
                },
            },
        }
    }
}

impl AppConfig {
    pub fn load_from_toml(path: impl AsRef<Path>) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref())
            .with_context(|| format!("read config {}", path.as_ref().display()))?;
        let mut cfg: Self = toml::from_str(&text)?;
        cfg.apply_env_overrides();
        Ok(cfg)
    }

    pub fn load_default_or_builtin(repo_config: impl AsRef<Path>) -> Self {
        match Self::load_from_toml(repo_config.as_ref()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "使用内置默认配置（未能读 {}: {e:#})",
                    repo_config.as_ref().display()
                );
                let mut c = Self::default();
                c.apply_env_overrides();
                c
            }
        }
    }

    /// 环境变量覆盖敏感项（不写进仓库）
    pub fn apply_env_overrides(&mut self) {
        if let Ok(tok) = env::var("JIAN_ACCESS_TOKEN") {
            if !tok.trim().is_empty() {
                self.sources.jian.access_token = Some(tok.trim().to_string());
            }
        }
    }

    pub fn user_config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("top", "sismotide", "jian-project")
            .map(|d| d.config_dir().to_path_buf())
    }

    pub fn any_source_enabled(&self) -> bool {
        self.sources.jian.enabled || self.sources.wolfx.enabled || self.sources.p2pquake.enabled
    }
}
