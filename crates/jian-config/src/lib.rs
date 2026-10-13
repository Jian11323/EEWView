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
    #[serde(default)]
    pub performance: PerformanceConfig,
    /// 主窗口位置与大小（调整后自动保存）
    #[serde(default)]
    pub window: WindowConfig,
    pub map: MapConfig,
    pub audio: AudioConfig,
    pub travel: TravelConfig,
    pub local: LocalConfig,
    pub sources: SourcesConfig,
    /// 速报震级阈值与地点白名单
    #[serde(default)]
    pub filters: FiltersConfig,
    /// 文件日志（便携版无控制台时仍写入资源根下 logs/）
    #[serde(default)]
    pub log: LogConfig,
}

/// 运行日志：等级、目录、是否同时打 stderr、保留天数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// 是否写文件（关闭则仅依赖 `RUST_LOG` + 控制台层，若有）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `error` | `warn` | `info` | `debug` | `trace`（可被环境变量 `RUST_LOG` 覆盖）
    #[serde(default = "default_log_level")]
    pub level: String,
    /// 相对资源根或绝对路径；默认 `logs`
    #[serde(default = "default_log_dir")]
    pub dir: String,
    /// 是否同时输出到 stderr（release 便携默认关；`cargo run` debug 构建仍会打控制台）
    #[serde(default)]
    pub console: bool,
    /// 按修改时间清理旧日志文件的保留天数（0=不清理）
    #[serde(default = "default_log_keep_days")]
    pub keep_days: u64,
}

fn default_log_level() -> String {
    "info".into()
}
fn default_log_dir() -> String {
    "logs".into()
}
fn default_log_keep_days() -> u64 {
    14
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: default_log_level(),
            dir: default_log_dir(),
            console: false,
            keep_days: default_log_keep_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub theme: String,
    pub show_nied_clock: bool,
    pub sidebar_default_tab: String,
    pub intensity_scale: String,
    /// 主窗口始终置顶（对齐滚动字幕「窗口置顶」）
    #[serde(default)]
    pub always_on_top: bool,
    /// 新预警/报数更新时地图自动跟焦震中
    #[serde(default = "default_true")]
    pub auto_follow_eew: bool,
    /// 跟焦时按 P/S 波圈自动缩放
    #[serde(default = "default_true")]
    pub auto_zoom_waves: bool,
    /// 绘制 P/S 走时波圈
    #[serde(default = "default_true")]
    pub show_wave_rings: bool,
    /// 绘制衰减估算烈度/震度同心环
    #[serde(default = "default_true")]
    pub show_intensity_rings: bool,
    /// 地图左下角烈度图例
    #[serde(default = "default_true")]
    pub show_legend: bool,
    /// 地图绘制火山三角标，并在火山列表显示图标
    #[serde(default = "default_true")]
    pub show_volcanoes: bool,
    /// 地图绘制台风路径 / 风圈 / 预报
    #[serde(default = "default_true")]
    pub show_typhoons: bool,
    /// 预报机构：全部 / 中国 / 中国台湾 / 日本 / 美国 / 中国香港
    #[serde(default = "default_typhoon_forecast_agency")]
    pub typhoon_forecast_agency: String,
}


/// 主窗口几何；缺省由系统摆放，用户拖动/缩放后写入用户配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    #[serde(default = "default_window_width")]
    pub width: f32,
    #[serde(default = "default_window_height")]
    pub height: f32,
    /// 外框左上角 X（物理像素近似）；缺省不强制
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
}

fn default_window_width() -> f32 {
    1280.0
}
fn default_window_height() -> f32 {
    720.0
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: default_window_width(),
            height: default_window_height(),
            x: None,
            y: None,
            maximized: false,
        }
    }
}

/// 渲染设备与性能档位（改渲染后端需重启后生效）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// `cpu` = 软渲染；`gpu` = 硬件加速
    #[serde(default = "default_perf_device")]
    pub device: String,
    /// GPU API：`opengl` | `vulkan`（仅 device=gpu）
    #[serde(default = "default_perf_gpu_api")]
    pub gpu_api: String,
    /// `eco` 节能 | `normal` 正常 | `extreme` 极致
    #[serde(default = "default_perf_profile")]
    pub profile: String,
    /// 目标帧率（空闲/波圈重绘）；默认 30，范围 20–300
    #[serde(default = "default_target_fps")]
    pub target_fps: u32,
}

fn default_perf_device() -> String {
    "gpu".into()
}
fn default_perf_gpu_api() -> String {
    "opengl".into()
}
fn default_perf_profile() -> String {
    "normal".into()
}
fn default_target_fps() -> u32 {
    30
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            device: default_perf_device(),
            gpu_api: default_perf_gpu_api(),
            profile: default_perf_profile(),
            target_fps: default_target_fps(),
        }
    }
}

impl PerformanceConfig {
    /// 钳位到 20–300。
    pub fn clamped_fps(&self) -> u32 {
        self.target_fps.clamp(20, 300)
    }

    /// 目标帧率对应的重绘间隔（毫秒）。
    pub fn repaint_interval_ms(&self) -> u64 {
        let fps = self.clamped_fps().max(1);
        (1000 / fps).max(1) as u64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapConfig {
    /// 底图：`vector`（默认 GeoJSON）| `tiles`（XYZ 瓦片）| `both`（瓦片+矢量边界）
    #[serde(default = "default_basemap")]
    pub basemap: String,
    /// 投影：`flat`（平面）| `globe`（球体）
    #[serde(default = "default_projection")]
    pub projection: String,
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
fn default_projection() -> String {
    "flat".into()
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
    /// Windows TTS 语音播报（预警/速报文案）
    #[serde(default)]
    pub tts_enabled: bool,
}

/// 自定义震级区间 → 最大 S 波传播距离。
/// 兼容旧键 `magnitude`（视为最小震级，最大默认为 9.5）。
#[derive(Debug, Clone, Serialize)]
pub struct MagSpreadPoint {
    pub min_magnitude: f64,
    pub max_magnitude: f64,
    pub max_km: f64,
}

impl<'de> serde::Deserialize<'de> for MagSpreadPoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Raw {
            min_magnitude: Option<f64>,
            max_magnitude: Option<f64>,
            magnitude: Option<f64>,
            max_km: f64,
        }
        let r = Raw::deserialize(deserializer)?;
        let min_m = r
            .min_magnitude
            .or(r.magnitude)
            .ok_or_else(|| serde::de::Error::missing_field("min_magnitude"))?;
        let max_m = r.max_magnitude.unwrap_or(9.5);
        Ok(Self {
            min_magnitude: min_m,
            max_magnitude: max_m,
            max_km: r.max_km,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelConfig {
    pub primary: String,
    pub fallback: String,
    pub linear_last_resort: bool,
    /// 震级区间→最大 S 波传播距离（km）；空则用内置表
    #[serde(default)]
    pub mag_spread_km: Vec<MagSpreadPoint>,
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
    /// PancakesAPI（api.aloys23.link）全灾害源
    #[serde(default)]
    pub pancakes: PancakesSourceConfig,
    /// 测站网：JMA（kmoni/lmoni/中转）、S-Net、KMA
    #[serde(default)]
    pub stations: StationsConfig,
    /// 活跃台风路径
    #[serde(default)]
    pub typhoon: TyphoonSourceConfig,
}

/// 台风实时路径（HTTP 轮询）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TyphoonSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_typhoon_url")]
    pub url: String,
    #[serde(default = "default_typhoon_poll")]
    pub poll_secs: u64,
}

impl Default for TyphoonSourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: default_typhoon_url(),
            poll_secs: default_typhoon_poll(),
        }
    }
}


/// PancakesAPI：仅 GlobalQuake（gq）与 JMA-Atom（上游 source=`jma_volcano`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PancakesSourceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_pancakes_base")]
    pub base_url: String,
    #[serde(default = "default_pancakes_ws")]
    pub ws_url: String,
    #[serde(default = "default_pancakes_client")]
    pub client_id: String,
    /// GlobalQuake 最低震级（0=不过滤）
    #[serde(default = "default_gq_min_mag")]
    pub gq_min_magnitude: f64,
    #[serde(default = "default_true")]
    pub gq: bool,
    /// JMA-Atom（火山情报；上游 `jma_volcano`）
    #[serde(default = "default_true", alias = "jma_atom")]
    pub jma_volcano: bool,
}

impl Default for PancakesSourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: default_pancakes_base(),
            ws_url: default_pancakes_ws(),
            client_id: default_pancakes_client(),
            gq_min_magnitude: default_gq_min_mag(),
            gq: true,
            jma_volcano: true,
        }
    }
}

/// 实时测站源选择（用户可切换原版 / Jian 中转）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationsConfig {
    /// 日本 JMA/NIED 测站总开关
    #[serde(default = "default_true")]
    pub jma_enabled: bool,
    /// `jian` | `kmoni` | `lmoni`
    #[serde(default = "default_jma_station_source")]
    pub jma_source: String,
    #[serde(default = "default_jian_station_ws")]
    pub jma_jian_ws: String,
    /// S-Net 海上强震网
    #[serde(default)]
    pub snet_enabled: bool,
    /// `jian` | `original`
    #[serde(default = "default_jma_station_source")]
    pub snet_source: String,
    #[serde(default = "default_snet_jian_ws")]
    pub snet_jian_ws: String,
    /// 韩国 KMA PEWS（仅 Jian API）
    #[serde(default)]
    pub kma_enabled: bool,
    #[serde(default = "default_kma_station_ws")]
    pub kma_jian_ws: String,
}

impl Default for StationsConfig {
    fn default() -> Self {
        Self {
            jma_enabled: true,
            jma_source: default_jma_station_source(),
            jma_jian_ws: default_jian_station_ws(),
            snet_enabled: false,
            snet_source: default_jma_station_source(),
            snet_jian_ws: default_snet_jian_ws(),
            kma_enabled: true,
            kma_jian_ws: default_kma_station_ws(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JianSourceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `/all` 预警总开关
    #[serde(default = "default_true")]
    pub eew_enabled: bool,
    /// `/all` 速报总开关
    #[serde(default = "default_true")]
    pub record_enabled: bool,
    #[serde(default = "default_true")]
    pub jma_eew: bool,
    #[serde(default = "default_true")]
    pub cea_eew: bool,
    #[serde(default = "default_true")]
    pub cwa_eew: bool,
    #[serde(default = "default_true")]
    pub kma_eew: bool,
    #[serde(default = "default_true")]
    pub sa_eew: bool,
    #[serde(default = "default_true")]
    pub early_est_eew: bool,
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
    /// Wolfx `all_eew` 预警总开关
    #[serde(default = "default_true")]
    pub eew_enabled: bool,
    /// Wolfx 速报列表总开关（WS query + HTTP）
    #[serde(default = "default_true")]
    pub eqlist_enabled: bool,
    #[serde(default = "default_true")]
    pub jma_eew: bool,
    /// 四川 EEW（cea-pr）；默认关，避免与 Jian 省级源重复
    #[serde(default = "default_false")]
    pub sc_eew: bool,
    #[serde(default = "default_false")]
    pub fj_eew: bool,
    #[serde(default = "default_false")]
    pub cq_eew: bool,
    /// Wolfx CENC 国网预警（入库为 wolfx-cea，展示 [CN]）
    #[serde(default = "default_true")]
    pub cenc_eew: bool,
    #[serde(default = "default_false")]
    pub cwa_eew: bool,
    #[serde(default = "default_true")]
    pub cenc_eqlist: bool,
    #[serde(default = "default_true")]
    pub jma_eqlist: bool,
    #[serde(default = "default_wolfx_ws")]
    pub ws_urls: Vec<String>,
    /// 冷启动速报列表（HTTP JSON）
    #[serde(default = "default_wolfx_eqlist")]
    pub http_eqlist: String,
}

impl WolfxSourceConfig {
    pub fn type_enabled(&self, typ: &str) -> bool {
        // 仅看各子源开关；总开关字段保留兼容旧配置，不再参与门控
        match typ {
            "jma_eew" => self.jma_eew,
            "sc_eew" => self.sc_eew,
            "fj_eew" => self.fj_eew,
            "cq_eew" => self.cq_eew,
            "cenc_eew" => self.cenc_eew,
            "cwa_eew" => self.cwa_eew,
            "cenc_eqlist" => self.cenc_eqlist,
            "jma_eqlist" => self.jma_eqlist,
            _ => true,
        }
    }

    pub fn any_eew(&self) -> bool {
        self.jma_eew || self.sc_eew || self.fj_eew || self.cq_eew || self.cenc_eew || self.cwa_eew
    }

    pub fn any_eqlist(&self) -> bool {
        self.cenc_eqlist || self.jma_eqlist
    }
}

/// 速报过滤：源开关与震级阈值分离。
///
/// - `record_enabled`：各机构速报开关（关则整源丢弃）
/// - `record_mag`：`0` 不过滤；`>0` 最低震级。白名单可绕过震级，不能绕过关闭的源。
/// - 兼容旧配置：`record_mag = -1` 视为该源关闭，并按 `0` 读阈值。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FiltersConfig {
    /// 各机构速报开关；缺省：`other` 关，其余开
    #[serde(default)]
    pub record_enabled: HashMap<String, bool>,
    #[serde(default)]
    pub record_mag: HashMap<String, f64>,
    #[serde(default)]
    pub place_whitelist: String,
}

impl FiltersConfig {
    /// 该机构速报是否接收（与震级阈值无关）。
    pub fn record_source_enabled(&self, key: &str) -> bool {
        if let Some(on) = self.record_enabled.get(key) {
            return *on;
        }
        // 旧配置用 mag=-1 表示关闭
        if self.record_mag.get(key).copied().unwrap_or(0.0) < 0.0 {
            return false;
        }
        key != "other"
    }

    pub fn set_record_source_enabled(&mut self, key: &str, on: bool) {
        self.record_enabled.insert(key.to_string(), on);
    }

    /// `0` 不过滤；正数为最低震级。负值（旧「不接收」）按 `0` 处理。
    pub fn record_mag_threshold(&self, key: &str) -> f64 {
        let v = self.record_mag.get(key).copied().unwrap_or(0.0);
        if v < 0.0 {
            0.0
        } else {
            v
        }
    }

    pub fn set_record_mag(&mut self, key: &str, val: f64) {
        self.record_mag.insert(key.to_string(), val.max(0.0));
    }

    /// 将旧 `record_mag=-1` 迁移为源开关关闭，避免与震级过滤混用。
    pub fn migrate_legacy_record_off(&mut self) {
        let keys: Vec<String> = self.record_mag.keys().cloned().collect();
        for key in keys {
            if self.record_mag.get(&key).copied().unwrap_or(0.0) < 0.0 {
                self.record_enabled.entry(key.clone()).or_insert(false);
                self.record_mag.insert(key, 0.0);
            }
        }
        self.record_enabled.entry("other".into()).or_insert(false);
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
fn default_false() -> bool {
    false
}
fn default_jian_ws() -> String {
    "wss://api.sismotide.top/all".into()
}
fn default_jian_station_ws() -> String {
    "wss://api.sismotide.top/kmoni".into()
}
fn default_jma_station_source() -> String {
    "jian".into()
}
fn default_snet_jian_ws() -> String {
    "wss://api.sismotide.top/s-net".into()
}
fn default_kma_station_ws() -> String {
    "wss://api.sismotide.top/kma-station".into()
}
fn default_wolfx_ws() -> Vec<String> {
    vec!["wss://ws-api.wolfx.jp/all_eew".into()]
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
fn default_pancakes_base() -> String {
    "https://api.aloys23.link".into()
}
fn default_pancakes_ws() -> String {
    "wss://api.aloys23.link".into()
}
fn default_pancakes_client() -> String {
    "eewview".into()
}
fn default_gq_min_mag() -> f64 {
    3.0
}
fn default_typhoon_url() -> String {
    "https://api.sismotide.top/get/typhoon.php".into()
}
fn default_typhoon_poll() -> u64 {
    15
}
fn default_typhoon_forecast_agency() -> String {
    "中国".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig {
                theme: "dark".into(),
                show_nied_clock: false,
                sidebar_default_tab: "eew".into(),
                intensity_scale: "auto".into(),
                always_on_top: false,
                auto_follow_eew: true,
                auto_zoom_waves: true,
                show_wave_rings: true,
                show_intensity_rings: true,
                show_legend: true,
                show_volcanoes: true,
                show_typhoons: true,
                typhoon_forecast_agency: default_typhoon_forecast_agency(),
            },
            performance: PerformanceConfig::default(),
            window: WindowConfig::default(),
            map: MapConfig {
                basemap: default_basemap(),
                projection: default_projection(),
                geodata_dir: default_geodata_dir(),
                tile_base: "https://tilemap.sismotide.top".into(),
                tile_source: "arcwob".into(),
                default_lon: 120.0,
                default_lat: 30.0,
                default_zoom: 6.8,
            },
            audio: AudioConfig {
                event_pack: "srev".into(),
                countdown_pack: "countdown".into(),
                master_volume: 0.8,
                countdown_volume: 1.0,
                mute: false,
                tts_enabled: false,
            },
            travel: TravelConfig {
                primary: "jma2001".into(),
                fallback: "ak135".into(),
                linear_last_resort: true,
                mag_spread_km: Vec::new(),
            },
            local: LocalConfig {
                latitude: None,
                longitude: None,
            },
            sources: SourcesConfig {
                jian: JianSourceConfig {
                    enabled: true,
                    eew_enabled: true,
                    record_enabled: true,
                    jma_eew: true,
                    cea_eew: true,
                    cwa_eew: true,
                    kma_eew: true,
                    sa_eew: true,
                    early_est_eew: true,
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
                    jma_eew: true,
                    sc_eew: false,
                    fj_eew: false,
                    cq_eew: false,
                    cenc_eew: true,
                    cwa_eew: false,
                    cenc_eqlist: true,
                    jma_eqlist: true,
                    ws_urls: default_wolfx_ws(),
                    http_eqlist: default_wolfx_eqlist(),
                },
                p2pquake: P2pSourceConfig {
                    enabled: true,
                    http_history: default_p2p_http(),
                    ws_url: default_p2p_ws(),
                    poll_secs: default_p2p_poll(),
                },
                pancakes: PancakesSourceConfig::default(),
                stations: StationsConfig::default(),
                typhoon: TyphoonSourceConfig::default(),
            },
            filters: FiltersConfig::default(),
            log: LogConfig::default(),
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
    /// Windows 用户覆盖：`%APPDATA%\eew view\settings.json`。
    pub fn load_merged(repo_config: impl AsRef<Path>) -> Self {
        let mut cfg = Self::load_default_or_builtin(repo_config);
        if let Some(path) = Self::user_config_path() {
            if path.is_file() {
                match Self::apply_user_overlay_file(&mut cfg, &path) {
                    Ok(()) => tracing_log(&format!("已合并用户配置 {}", path.display())),
                    Err(e) => eprintln!("用户配置无效，已忽略 {}: {e:#}", path.display()),
                }
            } else {
                // 一次性迁移旧路径（sismotide / config.toml）
                for legacy in Self::legacy_user_config_paths() {
                    if legacy.is_file() {
                        match Self::apply_user_overlay_file(&mut cfg, &legacy) {
                            Ok(()) => {
                                tracing_log(&format!(
                                    "已从旧配置迁移 {} → settings.json",
                                    legacy.display()
                                ));
                                let _ = cfg.save_user_overlay();
                                break;
                            }
                            Err(e) => eprintln!("旧配置无法读取 {}: {e:#}", legacy.display()),
                        }
                    }
                }
            }
        }
        cfg.apply_env_overrides();
        cfg.filters.migrate_legacy_record_off();
        cfg
    }

    fn apply_user_overlay_file(cfg: &mut Self, path: &Path) -> Result<()> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.ends_with(".json") {
            let overlay: serde_json::Value =
                serde_json::from_str(&text).context("parse settings.json")?;
            let base = serde_json::to_value(&*cfg).context("serialize base config")?;
            let merged = merge_json(base, overlay);
            *cfg = serde_json::from_value(merged).context("settings.json → AppConfig")?;
        } else {
            let overlay: toml::Value = text.parse().context("parse TOML overlay")?;
            let base = toml::Value::try_from(&*cfg).context("serialize base config")?;
            let merged = merge_toml(base, overlay);
            *cfg = merged.try_into().context("TOML overlay → AppConfig")?;
        }
        Ok(())
    }

    pub fn user_config_path() -> Option<PathBuf> {
        Self::user_config_dir().map(|d| d.join("settings.json"))
    }

    /// 写入用户目录覆盖为 `settings.json`（不含短期 `at_`；保留长期 `rt_`）。
    pub fn save_user_overlay(&self) -> Result<PathBuf> {
        let dir = Self::user_config_dir().context("无法解析用户配置目录")?;
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join("settings.json");
        let mut sanitized = self.clone();
        sanitized.filters.migrate_legacy_record_off();
        sanitized.sources.jian.access_token = None;
        let text = serde_json::to_string_pretty(&sanitized).context("serialize settings.json")?;
        fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }

    /// `stations.jma_*` 为准；旧键 `jian.station_enabled` 仅镜像，不得反杀测站开关。
    pub fn sync_station_compat(&mut self) {
        if self.sources.stations.jma_enabled && self.sources.stations.jma_source == "jian" {
            self.sources.jian.station_enabled = true;
            self.sources.jian.station_ws_url = self.sources.stations.jma_jian_ws.clone();
        } else if self.sources.stations.jma_source == "jian" {
            self.sources.jian.station_enabled = false;
        } else {
            // 原版 kmoni/lmoni：关闭 Jian /kmoni 连接
            self.sources.jian.station_enabled = false;
        }
    }

    /// 环境变量覆盖敏感项（不写进仓库）
    pub fn apply_env_overrides(&mut self) {
        self.sync_station_compat();
        self.filters.migrate_legacy_record_off();
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

    /// 用户数据根目录。
    /// Windows：`%APPDATA%\eew view`（配置 `settings.json`、日志 `log.txt`）。
    pub fn user_config_dir() -> Option<PathBuf> {
        Self::user_app_dir()
    }

    /// 与 `user_config_dir` 相同（本机可写用户数据）。
    pub fn user_data_local_dir() -> Option<PathBuf> {
        Self::user_app_dir()
    }

    /// 平台用户目录：`eew view`。
    pub fn user_app_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            env::var_os("APPDATA").map(|p| PathBuf::from(p).join("eew view"))
        }
        #[cfg(not(windows))]
        {
            directories::BaseDirs::new().map(|b| b.config_dir().join("eew view"))
        }
    }

    /// 旧版配置路径（仅迁移读取，不再写入）。
    fn legacy_user_config_paths() -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(proj) = directories::ProjectDirs::from("top", "sismotide", "jian-project") {
            let base = proj.config_dir().to_path_buf();
            out.push(base.join("config.toml"));
            out.push(base.join("config").join("config.toml"));
            out.push(base.join("settings.json"));
        }
        out
    }

    pub fn any_source_enabled(&self) -> bool {
        let st = &self.sources.stations;
        self.sources.jian.enabled
            || self.sources.jian.station_enabled
            || st.jma_enabled
            || st.snet_enabled
            || st.kma_enabled
            || (self.sources.wolfx.enabled
                && (self.sources.wolfx.any_eew() || self.sources.wolfx.any_eqlist()))
            || self.sources.p2pquake.enabled
            || self.sources.pancakes.enabled
            || self.sources.typhoon.enabled
    }

    pub fn sources_runtime_equal(&self, other: &Self) -> bool {
        self.sources.jian.enabled == other.sources.jian.enabled
            && self.sources.jian.eew_enabled == other.sources.jian.eew_enabled
            && self.sources.jian.record_enabled == other.sources.jian.record_enabled
            && self.sources.jian.jma_eew == other.sources.jian.jma_eew
            && self.sources.jian.cea_eew == other.sources.jian.cea_eew
            && self.sources.jian.cwa_eew == other.sources.jian.cwa_eew
            && self.sources.jian.kma_eew == other.sources.jian.kma_eew
            && self.sources.jian.sa_eew == other.sources.jian.sa_eew
            && self.sources.jian.early_est_eew == other.sources.jian.early_est_eew
            && self.sources.jian.station_enabled == other.sources.jian.station_enabled
            && self.sources.jian.ws_url == other.sources.jian.ws_url
            && self.sources.jian.station_ws_url == other.sources.jian.station_ws_url
            && self.sources.jian.refresh_token == other.sources.jian.refresh_token
            && self.sources.jian.access_token == other.sources.jian.access_token
            && self.sources.stations.jma_enabled == other.sources.stations.jma_enabled
            && self.sources.stations.jma_source == other.sources.stations.jma_source
            && self.sources.stations.jma_jian_ws == other.sources.stations.jma_jian_ws
            && self.sources.stations.snet_enabled == other.sources.stations.snet_enabled
            && self.sources.stations.snet_source == other.sources.stations.snet_source
            && self.sources.stations.snet_jian_ws == other.sources.stations.snet_jian_ws
            && self.sources.stations.kma_enabled == other.sources.stations.kma_enabled
            && self.sources.stations.kma_jian_ws == other.sources.stations.kma_jian_ws
            && self.sources.wolfx.enabled == other.sources.wolfx.enabled
            && self.sources.wolfx.eew_enabled == other.sources.wolfx.eew_enabled
            && self.sources.wolfx.eqlist_enabled == other.sources.wolfx.eqlist_enabled
            && self.sources.wolfx.jma_eew == other.sources.wolfx.jma_eew
            && self.sources.wolfx.sc_eew == other.sources.wolfx.sc_eew
            && self.sources.wolfx.fj_eew == other.sources.wolfx.fj_eew
            && self.sources.wolfx.cq_eew == other.sources.wolfx.cq_eew
            && self.sources.wolfx.cenc_eew == other.sources.wolfx.cenc_eew
            && self.sources.wolfx.cwa_eew == other.sources.wolfx.cwa_eew
            && self.sources.wolfx.cenc_eqlist == other.sources.wolfx.cenc_eqlist
            && self.sources.wolfx.jma_eqlist == other.sources.wolfx.jma_eqlist
            && self.sources.wolfx.ws_urls == other.sources.wolfx.ws_urls
            && self.sources.wolfx.http_eqlist == other.sources.wolfx.http_eqlist
            && self.sources.p2pquake.enabled == other.sources.p2pquake.enabled
            && self.sources.p2pquake.http_history == other.sources.p2pquake.http_history
            && self.sources.p2pquake.ws_url == other.sources.p2pquake.ws_url
            && self.sources.p2pquake.poll_secs == other.sources.p2pquake.poll_secs
            && self.sources.pancakes.enabled == other.sources.pancakes.enabled
            && self.sources.pancakes.base_url == other.sources.pancakes.base_url
            && self.sources.pancakes.ws_url == other.sources.pancakes.ws_url
            && self.sources.pancakes.client_id == other.sources.pancakes.client_id
            && self.sources.pancakes.gq_min_magnitude == other.sources.pancakes.gq_min_magnitude
            && self.sources.pancakes.gq == other.sources.pancakes.gq
            && self.sources.pancakes.jma_volcano == other.sources.pancakes.jma_volcano
            && self.sources.typhoon.enabled == other.sources.typhoon.enabled
            && self.sources.typhoon.url == other.sources.typhoon.url
            && self.sources.typhoon.poll_secs == other.sources.typhoon.poll_secs
    }
}

fn merge_json(base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    match (base, overlay) {
        (serde_json::Value::Object(mut a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                match a.remove(&k) {
                    Some(prev) => {
                        a.insert(k, merge_json(prev, v));
                    }
                    None => {
                        a.insert(k, v);
                    }
                }
            }
            serde_json::Value::Object(a)
        }
        (_, overlay) => overlay,
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
