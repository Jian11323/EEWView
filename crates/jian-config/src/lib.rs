//! 配置加载。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// 预警机构开关、速报震级阈值（对齐 RhythmQuake）
    #[serde(default)]
    pub filters: FiltersConfig,
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
    /// 底图：`vector`（默认 GeoJSON）| `tiles`（XYZ 瓦片）| `both`（瓦片+矢量边界）
    #[serde(default = "default_basemap")]
    pub basemap: String,
    /// 相对仓库根或绝对路径；`vector` / `both` 时加载
    #[serde(default = "default_geodata_dir")]
    pub geodata_dir: String,
    pub tile_base: String,
    pub tile_source: String,
    pub default_lon: f64,
    pub default_lat: f64,
    pub default_zoom: f64,
}

fn default_basemap() -> String {
    "auto".into()
}
fn default_geodata_dir() -> String {
    "geodata".into()
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
    /// 短期访问令牌 `at_…`（运行时 / 环境变量；不写入用户覆盖）
    #[serde(default)]
    pub access_token: Option<String>,
    /// 长期 Token `rt_…`（设置页用 lk_ 换取后写入用户目录，约 180 天）
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// 长期 Token 过期 unix 秒；0 表示未知
    #[serde(default)]
    pub refresh_expire_at: f64,
    /// NIED Kmoni 测站（无需令牌，独立连接）
    #[serde(default = "default_true")]
    pub station_enabled: bool,
    #[serde(default = "default_jian_station_ws")]
    pub station_ws_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WolfxSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Wolfx JMA EEW WebSocket
    #[serde(default = "default_true")]
    pub eew_enabled: bool,
    /// Wolfx CENC 速报列表
    #[serde(default = "default_true")]
    pub eqlist_enabled: bool,
    #[serde(default = "default_wolfx_ws")]
    pub ws_urls: Vec<String>,
    /// 冷启动速报列表（HTTP JSON）
    #[serde(default = "default_wolfx_eqlist")]
    pub http_eqlist: String,
}

/// `-1` 不接收；`0` 不过滤；`>0` 最低震级。白名单可绕过震级，但不能绕过「不接收」。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FiltersConfig {
    #[serde(default)]
    pub eew_agency: HashMap<String, bool>,
    #[serde(default)]
    pub record_mag: HashMap<String, f64>,
    #[serde(default)]
    pub place_whitelist: String,
}

impl FiltersConfig {
    pub fn eew_agency_on(&self, key: &str) -> bool {
        self.eew_agency.get(key).copied().unwrap_or(true)
    }

    pub fn set_eew_agency(&mut self, key: &str, on: bool) {
        self.eew_agency.insert(key.to_string(), on);
    }

    /// `-1` 不接收；`0` 不过滤；正数为最低震级。未适配机构默认不接收。
    pub fn record_mag_threshold(&self, key: &str) -> f64 {
        self.record_mag
            .get(key)
            .copied()
            .unwrap_or(if key == "other" { -1.0 } else { 0.0 })
    }

    pub fn set_record_mag(&mut self, key: &str, val: f64) {
        self.record_mag.insert(key.to_string(), val);
    }

    pub fn place_whitelist_hit(&self, place: &str) -> bool {
        let raw = self.place_whitelist.trim();
        if raw.is_empty() {
            return false;
        }
        raw.split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|kw| place.contains(kw))
    }
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
fn default_jian_station_ws() -> String {
    "wss://api.sismotide.top/kmoni".into()
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
                basemap: default_basemap(),
                geodata_dir: default_geodata_dir(),
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
                    refresh_token: None,
                    refresh_expire_at: 0.0,
                    station_enabled: true,
                    station_ws_url: default_jian_station_ws(),
                },
                wolfx: WolfxSourceConfig {
                    enabled: true,
                    eew_enabled: true,
                    eqlist_enabled: true,
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
            filters: FiltersConfig::default(),
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

    /// 仓库默认 + 用户目录覆盖 + 环境变量（令牌仍以环境变量优先）。
    pub fn load_merged(repo_config: impl AsRef<Path>) -> Self {
        let mut cfg = Self::load_default_or_builtin(repo_config);
        if let Some(path) = Self::user_config_path() {
            if path.is_file() {
                match fs::read_to_string(&path) {
                    Ok(text) => match text.parse::<toml::Value>() {
                        Ok(overlay) => match toml::Value::try_from(&cfg) {
                            Ok(base) => {
                                let merged = merge_toml(base, overlay);
                                match merged.try_into() {
                                    Ok(c) => {
                                        tracing_log(&format!(
                                            "已合并用户配置 {}",
                                            path.display()
                                        ));
                                        cfg = c;
                                    }
                                    Err(e) => eprintln!("用户配置无法解析，已忽略: {e:#}"),
                                }
                            }
                            Err(e) => eprintln!("序列化当前配置失败: {e:#}"),
                        },
                        Err(e) => eprintln!("用户配置 TOML 无效，已忽略: {e:#}"),
                    },
                    Err(e) => eprintln!("读取用户配置失败: {e:#}"),
                }
            }
        }
        cfg.apply_env_overrides();
        cfg
    }

    pub fn user_config_path() -> Option<PathBuf> {
        Self::user_config_dir().map(|d| d.join("config.toml"))
    }

    /// 写入用户目录覆盖（不含短期 `at_`；保留长期 `rt_`）。
    pub fn save_user_overlay(&self) -> Result<PathBuf> {
        let dir = Self::user_config_dir().context("无法解析用户配置目录")?;
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join("config.toml");
        let mut sanitized = self.clone();
        sanitized.sources.jian.access_token = None;
        let text = toml::to_string_pretty(&sanitized).context("serialize config")?;
        fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    /// 环境变量覆盖敏感项（不写进仓库）
    pub fn apply_env_overrides(&mut self) {
        if let Ok(tok) = env::var("JIAN_REFRESH_TOKEN") {
            if tok.trim().starts_with("rt_") {
                self.sources.jian.refresh_token = Some(tok.trim().to_string());
            }
        }
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
        self.sources.jian.enabled
            || self.sources.jian.station_enabled
            || (self.sources.wolfx.enabled
                && (self.sources.wolfx.eew_enabled || self.sources.wolfx.eqlist_enabled))
            || self.sources.p2pquake.enabled
    }

    pub fn sources_runtime_equal(&self, other: &Self) -> bool {
        self.sources.jian.enabled == other.sources.jian.enabled
            && self.sources.jian.station_enabled == other.sources.jian.station_enabled
            && self.sources.jian.ws_url == other.sources.jian.ws_url
            && self.sources.jian.station_ws_url == other.sources.jian.station_ws_url
            && self.sources.jian.refresh_token == other.sources.jian.refresh_token
            && self.sources.jian.access_token == other.sources.jian.access_token
            && self.sources.wolfx.enabled == other.sources.wolfx.enabled
            && self.sources.wolfx.eew_enabled == other.sources.wolfx.eew_enabled
            && self.sources.wolfx.eqlist_enabled == other.sources.wolfx.eqlist_enabled
            && self.sources.wolfx.ws_urls == other.sources.wolfx.ws_urls
            && self.sources.wolfx.http_eqlist == other.sources.wolfx.http_eqlist
            && self.sources.p2pquake.enabled == other.sources.p2pquake.enabled
            && self.sources.p2pquake.http_history == other.sources.p2pquake.http_history
            && self.sources.p2pquake.ws_url == other.sources.p2pquake.ws_url
            && self.sources.p2pquake.poll_secs == other.sources.p2pquake.poll_secs
    }
}

fn merge_toml(base: toml::Value, overlay: toml::Value) -> toml::Value {
    match (base, overlay) {
        (toml::Value::Table(mut a), toml::Value::Table(b)) => {
            for (k, v) in b {
                match a.remove(&k) {
                    Some(prev) => {
                        a.insert(k, merge_toml(prev, v));
                    }
                    None => {
                        a.insert(k, v);
                    }
                }
            }
            toml::Value::Table(a)
        }
        (_, overlay) => overlay,
    }
}

fn tracing_log(msg: &str) {
    eprintln!("{msg}");
}
