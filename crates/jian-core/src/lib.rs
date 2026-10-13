//! 领域模型与官方色标。

pub mod agency;
pub mod estimate;
pub mod intensity;
pub mod palette;
pub mod tts;

use serde::{Deserialize, Serialize};

pub use agency::{
    agency_bracket, agency_family, cea_pr_agency_id, cea_pr_province, eew_feed_priority,
    place_with_agency, AgencyFamily, EEW_FILTER_FAMILIES, RECORD_FILTER_FAMILIES,
};
pub use tts::{
    agency_tts_name, build_eew_tts_script, build_record_tts_script, should_play_eew_tts,
};
pub use estimate::{
    calc_csis, calc_jma_instrumental, estimate_at_site, estimate_epicentral, fill_intensity,
    hypocentral_km,
};
pub use intensity::{
    cn_intensity_level, cn_intensity_text, instrumental_display_text, instrumental_half_step_text,
    intensity_kind_for_agency,
    jma_from_instrumental, jma_shindo_level, jma_shindo_text, prefer_intensity_scale,
};
pub use palette::{instrumental_band_level, instrumental_band_rgb, instrumental_rgb};

// tsunami_official_rgb / tsunami_grade_zh 定义于本文件

/// 机构 / 源 ID（与 WS type 对齐）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgencyId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntensityKind {
    JmaShindo,
    CnIntensity,
}

/// 单次预警报文（归一化后）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EewReport {
    pub agency: AgencyId,
    pub event_id: String,
    pub serial: u32,
    pub place: String,
    pub magnitude: f64,
    pub depth_km: f64,
    pub latitude: f64,
    pub longitude: f64,
    /// 发震时刻 unix ms
    pub origin_ms: i64,
    /// 最大震度/烈度展示文案，如 "5弱" / "Ⅶ"
    pub max_intensity_text: String,
    pub intensity_kind: IntensityKind,
    /// 用于上色的档位索引（JMA: 0–9 对应 0～7；CN: 1–12）
    pub intensity_level: u8,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub is_cancel: bool,
    /// 仮定震源 / 调查中（Wolfx `isAssumption` 等）；不驱动本地倒计时
    #[serde(default)]
    pub is_assumption: bool,
    /// 紧急地震速报（警报）（Wolfx `isWarn` 或震度≥5弱）
    #[serde(default)]
    pub is_warn: bool,
    /// 收到最终报的本地时刻 unix ms（0=尚未最终）
    #[serde(default)]
    pub final_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EqRecord {
    pub agency: AgencyId,
    pub place: String,
    pub time_text: String,
    pub magnitude: f64,
    pub intensity_text: String,
    pub intensity_kind: IntensityKind,
    pub intensity_level: u8,
    pub latitude: f64,
    pub longitude: f64,
    pub depth_km: f64,
    /// 发震时刻 unix ms（0 表示未知）
    #[serde(default)]
    pub origin_ms: i64,
    /// CENC 等：自动测定（Wolfx `type=automatic`）
    #[serde(default)]
    pub is_auto: bool,
}

impl EqRecord {
    /// 供 Header / 地图标记展示（非实时 EEW，serial=0，不驱动正式倒计时语义）
    pub fn as_header_report(&self) -> EewReport {
        EewReport {
            agency: self.agency.clone(),
            event_id: format!("record:{}", self.place),
            serial: 0,
            place: self.place.clone(),
            magnitude: self.magnitude,
            depth_km: self.depth_km,
            latitude: self.latitude,
            longitude: self.longitude,
            origin_ms: self.origin_ms,
            max_intensity_text: self.intensity_text.clone(),
            intensity_kind: self.intensity_kind,
            intensity_level: self.intensity_level,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationSample {
    pub id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub intensity_text: String,
    pub intensity_kind: IntensityKind,
    pub intensity_level: u8,
    /// 計測震度（JMA 网约 -3～7；CN/KMA 可填烈度近似值；缺测用 -99）
    #[serde(default = "default_instrumental")]
    pub instrumental: f64,
}

fn default_instrumental() -> f64 {
    -99.0
}

/// 海啸影响区域一行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsunamiArea {
    pub name: String,
    /// Forecast / Watch / Warning / MajorWarning
    pub grade: String,
}

/// JMA 海啸情报（预报 / 注意报 / 警报等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsunamiInfo {
    /// 标题，如「海啸警报」「海啸注意报」「海啸预报」
    pub title: String,
    /// 等级原文或归一化：Forecast / Watch / Warning / MajorWarning / Cancel
    pub grade: String,
    /// 区域摘要（顿号分隔）
    pub areas: String,
    /// 分区明细（有则优先用于地图着色）
    #[serde(default)]
    pub areas_list: Vec<TsunamiArea>,
    pub cancelled: bool,
    /// 发布时间 unix ms（0 未知）
    #[serde(default)]
    pub issued_ms: i64,
    /// 来源标识：jian / pancakes / p2p
    #[serde(default)]
    pub source: String,
    /// 来源优先级：Jian=3 > Pancakes=2 > P2P=1（高者优先）
    #[serde(default)]
    pub source_priority: u8,
    /// 预报有效截止/预计解除 unix ms（0 未知）。注意报/警报级通常靠解除报文，不用此字段自动清。
    #[serde(default)]
    pub expire_ms: i64,
}

/// JMA 海啸四档官方色（预报青绿 / 注意报黄 / 警报红 / 大海啸紫）。
pub fn tsunami_official_rgb(grade: &str) -> (u8, u8, u8) {
    match grade {
        "MajorWarning" | "Major" => (0xC0, 0x00, 0xC0),
        "Warning" => (0xFF, 0x00, 0x00),
        "Watch" => (0xFF, 0xFF, 0x00),
        _ => (0x00, 0xE5, 0xFF), // Forecast 津波予報
    }
}

pub fn tsunami_grade_zh(grade: &str) -> &'static str {
    match grade {
        "MajorWarning" | "Major" => "大海啸警报",
        "Warning" => "海啸警报",
        "Watch" => "海啸注意报",
        "Cancel" => "海啸情报解除",
        _ => "海啸预报",
    }
}

/// JMA 火山情报（降灰预报等；不进地震速报列表）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolcanoNotice {
    pub agency: AgencyId,
    /// 火山名
    pub name: String,
    /// 标题，如「降灰予報（定時）」
    pub title: String,
    pub time_text: String,
    pub issued_ms: i64,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
}

impl StationSample {
    /// 测站选中时的 Header 展示（无震级/深度波圈语义）
    pub fn as_header_report(&self) -> EewReport {
        EewReport {
            agency: AgencyId("station".into()),
            event_id: format!("station:{}", self.id),
            serial: 0,
            place: self.name.clone(),
            magnitude: 0.0,
            depth_km: 0.0,
            latitude: self.latitude,
            longitude: self.longitude,
            origin_ms: 0,
            max_intensity_text: self.intensity_text.clone(),
            intensity_kind: self.intensity_kind,
            intensity_level: self.intensity_level,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// 正常
    Normal,
    /// 波动
    Fluctuating,
    /// 异常
    Abnormal,
}

impl HealthStatus {
    pub fn as_zh(self) -> &'static str {
        match self {
            Self::Normal => "正常",
            Self::Fluctuating => "波动",
            Self::Abnormal => "异常",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarTab {
    Records,
    #[default]
    Eew,
    Station,
    /// JMA 火山情报（与地震速报分离）
    Volcano,
}

/// 列表选中项（驱动高亮 + 地图居中）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSelection {
    pub tab: SidebarTab,
    pub index: usize,
}

/// 叠加层模式：EEW/速报画波圈；测站只画点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    Wave,
    MarkerOnly,
}

/// UI 用快照
#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub active: Option<EewReport>,
    pub overlay_mode: OverlayMode,
    pub records: Vec<EqRecord>,
    pub eew_list: Vec<EewReport>,
    pub stations: Vec<StationSample>,
    pub countdown_s: Option<i32>,
    /// 本机预估烈度/震度（与 countdown_s 同步；无倒计时时为空）
    pub countdown_local_kind: IntensityKind,
    pub countdown_local_level: u8,
    pub countdown_local_text: String,
    pub health_jian: HealthStatus,
    pub health_wolfx: HealthStatus,
    pub health_p2p: HealthStatus,
    pub health_pancakes: HealthStatus,
    pub tab: SidebarTab,
    pub selection: Option<ListSelection>,
    /// 图例 P-S：是否绘制走时波圈
    pub show_wave_rings: bool,
    /// 当前有效的 JMA 海啸情报（取消后为 None）
    pub tsunami: Option<TsunamiInfo>,
    /// JMA 火山情报（与 records 分离）
    pub volcanoes: Vec<VolcanoNotice>,
    /// 各测站网最近一帧到达时刻 unix ms（kmoni/snet/kma）
    pub station_updated_ms: std::collections::HashMap<String, i64>,
    /// 新速报写入后待主循环飞跃的 (lon, lat, zoom)；take 后清空
    pub pending_record_fly: Option<(f64, f64, f64)>,
    /// 待朗读速报（预警中不跳转时仍可 TTS；主循环 take）
    pub pending_tts_record: Option<EqRecord>,
    /// 自定义震级区间→最大传播距离(km)：(min_mag, max_mag, max_km)；空则用内置表
    pub wave_spread_km: Vec<(f64, f64, f64)>,
}

impl AppSnapshot {
    /// 无上游时的离线示例数据
    pub fn offline_sample() -> Self {
        let eew_a = EewReport {
            agency: AgencyId("jma-eew".into()),
            event_id: "sample-001".into(),
            serial: 3,
            place: "千叶县西北部".into(),
            magnitude: 5.4,
            depth_km: 40.0,
            latitude: 35.7,
            longitude: 140.1,
            origin_ms: 0,
            max_intensity_text: "5弱".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 5,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: true,
            final_at_ms: 0,
        };
        let eew_b = EewReport {
            agency: AgencyId("cea".into()),
            event_id: "sample-002".into(),
            serial: 2,
            place: "缅甸".into(),
            magnitude: 4.1,
            depth_km: 12.0,
            latitude: 22.0,
            longitude: 98.5,
            origin_ms: 0,
            max_intensity_text: "Ⅵ".into(),
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: 6,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        };
        let eew_pr = EewReport {
            agency: AgencyId("cea-pr:四川".into()),
            event_id: "sample-002b".into(),
            serial: 1,
            place: "四川阿坝州红原县".into(),
            magnitude: 4.4,
            depth_km: 5.0,
            latitude: 33.0,
            longitude: 102.9,
            origin_ms: 0,
            max_intensity_text: "Ⅵ".into(),
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: 6,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        };
        let eew_c = EewReport {
            agency: AgencyId("jma-eew".into()),
            event_id: "sample-003".into(),
            serial: 1,
            place: "宫城县冲".into(),
            magnitude: 6.1,
            depth_km: 50.0,
            latitude: 38.2,
            longitude: 142.0,
            origin_ms: 0,
            max_intensity_text: "5强".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 6,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: true,
            final_at_ms: 0,
        };

        Self {
            active: Some(eew_a.clone()),
            overlay_mode: OverlayMode::Wave,
            records: vec![
                EqRecord {
                    agency: AgencyId("cenc".into()),
                    place: "四川阿坝州汶川县".into(),
                    time_text: "2026-10-09 12:01".into(),
                    magnitude: 4.2,
                    intensity_text: "Ⅴ".into(),
                    intensity_kind: IntensityKind::CnIntensity,
                    intensity_level: 5,
                    latitude: 31.5,
                    longitude: 103.6,
                    depth_km: 12.0,
                    origin_ms: 0,
                    is_auto: false,
                },
                EqRecord {
                    agency: AgencyId("jma".into()),
                    place: "千叶县西北部".into(),
                    time_text: "2026-10-09 11:58".into(),
                    magnitude: 5.4,
                    intensity_text: "5弱".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 5,
                    latitude: 35.7,
                    longitude: 140.1,
                    depth_km: 40.0,
                    origin_ms: 0,
                    is_auto: false,
                },
                EqRecord {
                    agency: AgencyId("jma".into()),
                    place: "福岛县中通".into(),
                    time_text: "2026-10-09 09:12".into(),
                    magnitude: 4.6,
                    intensity_text: "4".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 4,
                    latitude: 37.4,
                    longitude: 140.4,
                    depth_km: 10.0,
                    origin_ms: 0,
                    is_auto: false,
                },
            ],
            eew_list: vec![eew_a, eew_b, eew_pr, eew_c],
            stations: vec![
                StationSample {
                    id: "TOKYO".into(),
                    name: "东京".into(),
                    latitude: 35.68,
                    longitude: 139.76,
                    intensity_text: "3".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 3,
                    instrumental: 3.2,
                },
                StationSample {
                    id: "OSAKA".into(),
                    name: "大阪".into(),
                    latitude: 34.69,
                    longitude: 135.50,
                    intensity_text: "2".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 2,
                    instrumental: 2.1,
                },
                StationSample {
                    id: "CD01".into(),
                    name: "成都".into(),
                    latitude: 30.67,
                    longitude: 104.06,
                    intensity_text: "Ⅳ".into(),
                    intensity_kind: IntensityKind::CnIntensity,
                    intensity_level: 4,
                    instrumental: 4.0,
                },
            ],
            countdown_s: Some(28),
            countdown_local_kind: IntensityKind::CnIntensity,
            countdown_local_level: 3,
            countdown_local_text: "3".into(),
            health_jian: HealthStatus::Normal,
            health_wolfx: HealthStatus::Normal,
            health_p2p: HealthStatus::Fluctuating,
            health_pancakes: HealthStatus::Abnormal,
            tab: SidebarTab::Eew,
            selection: Some(ListSelection {
                tab: SidebarTab::Eew,
                index: 0,
            }),
            show_wave_rings: true,
            tsunami: None,
            volcanoes: Vec::new(),
            station_updated_ms: std::collections::HashMap::new(),
            pending_record_fly: None,
            pending_tts_record: None,
            wave_spread_km: Vec::new(),
        }
    }

    /// 选中列表项 → 更新 Header / 叠加模式；返回应居中的 (lon, lat, 建议 zoom)
    pub fn select_list_item(&mut self, tab: SidebarTab, index: usize) -> Option<(f64, f64, f64)> {
        self.tab = tab;
        self.selection = Some(ListSelection { tab, index });
        match tab {
            SidebarTab::Eew => {
                let ev = self.eew_list.get(index)?.clone();
                let lon = ev.longitude;
                let lat = ev.latitude;
                self.active = Some(ev);
                self.overlay_mode = OverlayMode::Wave;
                Some((lon, lat, 8.0))
            }
            SidebarTab::Records => {
                let r = self.records.get(index)?;
                let lon = r.longitude;
                let lat = r.latitude;
                self.active = Some(r.as_header_report());
                self.overlay_mode = OverlayMode::Wave;
                Some((lon, lat, 8.0))
            }
            SidebarTab::Station => {
                let s = self.stations.get(index)?;
                let lon = s.longitude;
                let lat = s.latitude;
                self.active = Some(s.as_header_report());
                self.overlay_mode = OverlayMode::MarkerOnly;
                Some((lon, lat, 9.0))
            }
            SidebarTab::Volcano => {
                let v = self.volcanoes.get(index)?;
                let lon = v.longitude;
                let lat = v.latitude;
                // 火山不进左上预警条
                self.active = None;
                self.overlay_mode = OverlayMode::MarkerOnly;
                if lat.abs() <= 0.01 && lon.abs() <= 0.01 {
                    None
                } else {
                    Some((lon, lat, 8.0))
                }
            }
        }
    }

    /// 空快照（真数据模式起点；列表由网络事件填充）
    pub fn empty() -> Self {
        Self {
            active: None,
            overlay_mode: OverlayMode::Wave,
            records: Vec::new(),
            eew_list: Vec::new(),
            stations: Vec::new(),
            countdown_s: None,
            countdown_local_kind: IntensityKind::CnIntensity,
            countdown_local_level: 0,
            countdown_local_text: String::new(),
            health_jian: HealthStatus::Abnormal,
            health_wolfx: HealthStatus::Abnormal,
            health_p2p: HealthStatus::Abnormal,
            health_pancakes: HealthStatus::Abnormal,
            tab: SidebarTab::Eew,
            selection: None,
            show_wave_rings: true,
            tsunami: None,
            volcanoes: Vec::new(),
            station_updated_ms: std::collections::HashMap::new(),
            pending_record_fly: None,
            pending_tts_record: None,
            wave_spread_km: Vec::new(),
        }
    }

    pub fn set_tsunami(&mut self, info: Option<TsunamiInfo>) {
        self.tsunami = info;
    }

    /// 海啸并入：Jian > Pancakes > P2P；同级取更新的 issued_ms。
    /// 解除报文（注意报/警报等）直接清空；预报可带 expire_ms，到期由 `prune_expired_tsunami` 清除。
    pub fn upsert_tsunami(&mut self, info: TsunamiInfo) {
        let cancel = info.cancelled || info.grade.eq_ignore_ascii_case("Cancel");
        if cancel {
            if let Some(cur) = self.tsunami.as_ref() {
                if info.source_priority < cur.source_priority {
                    return;
                }
            }
            self.tsunami = None;
            return;
        }
        // 入库时已过期的预报直接丢弃，并清掉同级旧残留
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        if info.expire_ms > 0 && now_ms >= info.expire_ms {
            if let Some(cur) = self.tsunami.as_ref() {
                if info.source_priority >= cur.source_priority {
                    self.tsunami = None;
                }
            }
            return;
        }
        if let Some(cur) = self.tsunami.as_ref() {
            if info.source_priority < cur.source_priority {
                return;
            }
            if info.source_priority == cur.source_priority
                && info.issued_ms > 0
                && cur.issued_ms > info.issued_ms
            {
                return;
            }
        }
        self.tsunami = Some(info);
    }

    /// 清除已解除/已到期的海啸顶条。
    /// 预报：优先 `expire_ms`；缺截止时用 issued 后 2 小时兜底。注意报/警报等仍主要靠解除报文。
    pub fn prune_expired_tsunami(&mut self, now_ms: i64) {
        let Some(t) = self.tsunami.as_ref() else {
            return;
        };
        if t.cancelled || t.grade.eq_ignore_ascii_case("Cancel") {
            self.tsunami = None;
            return;
        }
        if t.expire_ms > 0 && now_ms >= t.expire_ms {
            self.tsunami = None;
            return;
        }
        let is_forecast = matches!(t.grade.as_str(), "Forecast" | "forecast" | "")
            || t.title.contains("预报")
            || t.title.contains("予報");
        if !is_forecast {
            return;
        }
        // 无解除时间的预报：避免永久残留
        let base = if t.issued_ms > 0 {
            t.issued_ms
        } else {
            return;
        };
        const FORECAST_TTL_MS: i64 = 2 * 60 * 60 * 1000;
        if now_ms >= base + FORECAST_TTL_MS {
            self.tsunami = None;
        }
    }

    pub fn set_health(&mut self, source: &str, status: HealthStatus) {
        match source {
            "jian" => self.health_jian = status,
            // 测站 /kmoni：仅测站通时标「波动」，避免无 /all 却显示 Jian「正常」
            "kmoni" => {
                if status == HealthStatus::Normal {
                    if self.health_jian == HealthStatus::Abnormal {
                        self.health_jian = HealthStatus::Fluctuating;
                    }
                } else if self.health_jian != HealthStatus::Normal {
                    self.health_jian = status;
                }
            }
            "wolfx" => self.health_wolfx = status,
            "p2pquake" | "p2p" => self.health_p2p = status,
            "pancakes" => self.health_pancakes = status,
            _ => {}
        }
    }

    /// 列表按龄裁剪（供回看）；不因波走完删记录。地图叉标另按 `eew_wave_active`。
    pub fn prune_finished_eews(&mut self) {
        // 每源最新一条常驻；其余按 14 天裁剪
        prune_eew_list(&mut self.eew_list, 14 * 24 * 60 * 60 * 1000, 80);
    }

    /// 插入/更新 EEW：跨 API 同机构族同一事件合并（如 Jian `jma-eew` 与 Wolfx `wolfx-jma-eew`）。
    /// 报数更高优先；报数相同取更高镜像源优先级。不同事件各留一条供多震波圈。
    /// 新预警：刷新左上角 + 切到预警 Tab；不改列表选中（与跟焦解绑，对齐 eewcn）。
    pub fn upsert_eew(&mut self, mut report: EewReport) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let pos = self
            .eew_list
            .iter()
            .position(|e| same_eew_event(e, &report));
        if let Some(pos) = pos {
            if !prefer_eew_report(&self.eew_list[pos], &report) {
                return;
            }
            let old = &self.eew_list[pos];
            // 保留已有最终报时间戳；新升最终报则打点
            if report.is_final {
                if old.final_at_ms > 0 {
                    report.final_at_ms = old.final_at_ms;
                } else if !old.is_final || report.final_at_ms <= 0 {
                    report.final_at_ms = now_ms;
                }
            } else {
                report.final_at_ms = 0;
            }
            self.eew_list[pos] = report.clone();
        } else {
            if report.is_final && report.final_at_ms <= 0 {
                report.final_at_ms = now_ms;
            }
            self.eew_list.push(report.clone());
        }
        sort_eew_newest_first(&mut self.eew_list);
        prune_eew_list(&mut self.eew_list, 14 * 24 * 60 * 60 * 1000, 80);
        if let Some(pos) = self
            .eew_list
            .iter()
            .position(|e| same_eew_event(e, &report))
        {
            self.active = Some(self.eew_list[pos].clone());
            self.overlay_mode = OverlayMode::Wave;
            self.tab = SidebarTab::Eew;
            // 取消尚未执行的速报飞跃，避免盖过预警跟焦
            self.pending_record_fly = None;
        }
    }

    /// 是否仍有传播中的真预警（按自定义传播表；表空则用内置近似）。
    fn has_live_eew(&self) -> bool {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.eew_list
            .iter()
            .any(|e| eew_wave_active_spread(e, now_ms, &self.wave_spread_km))
    }

    /// 插入速报；全源混合后按机构族+发震时刻去重；新→旧。
    /// 无传播中预警时：收到即选中并飞跃（不论是否列表最新）。
    /// 有传播中预警：只入列表不跳转、不改 active/Tab；符合条件时仍排队 TTS。
    pub fn upsert_record(&mut self, record: EqRecord) {
        // 火山不得混入地震速报
        if record.agency.0.eq_ignore_ascii_case("jma-volcano")
            || record.agency.0.to_ascii_lowercase().contains("volcano")
        {
            return;
        }
        let mut is_new_or_upgrade = true;
        let mut announce_tts = false;
        // 跨 API 同机构族同一事件：更新槽位（Jian cenc ↔ Wolfx wolfx-cenc；自动→正式）
        if let Some(pos) = self.records.iter().position(|r| same_record_event(r, &record)) {
            let cur = &self.records[pos];
            // 列表轮询/快照重放：内容未变则静默，避免反复 TTS
            if records_content_equivalent(cur, &record) {
                return;
            }
            let auto_to_final = cur.is_auto && !record.is_auto;
            let prefer_new = if auto_to_final {
                true
            } else if !cur.is_auto && record.is_auto {
                false
            } else {
                let pn = eew_feed_priority(&record.agency.0);
                let po = eew_feed_priority(&cur.agency.0);
                pn > po || (pn == po && record.origin_ms > cur.origin_ms)
            };
            if prefer_new {
                // 仅「自动→正式」或足够新的事件才朗读，避免历史列表刷屏
                announce_tts = auto_to_final || record_fresh_for_tts(record.origin_ms);
                self.records[pos] = record.clone();
                sort_records_newest_first(&mut self.records);
            } else {
                is_new_or_upgrade = false;
            }
        } else {
            self.records.push(record.clone());
            sort_records_newest_first(&mut self.records);
            if self.records.len() > 80 {
                self.records.truncate(80);
            }
            // 新条目：仅近时事件朗读（冷启动/定期 list 里的旧报不读）
            announce_tts = record_fresh_for_tts(record.origin_ms);
        }
        if !is_new_or_upgrade {
            return;
        }
        if announce_tts {
            self.pending_tts_record = Some(record.clone());
        }
        if self.has_live_eew() {
            return;
        }
        let lon = record.longitude;
        let lat = record.latitude;
        let has_coord = lat.abs() > 0.01 || lon.abs() > 0.01;
        if let Some(i) = self.records.iter().position(|r| {
            r.agency == record.agency
                && r.place == record.place
                && r.origin_ms == record.origin_ms
                && (r.magnitude - record.magnitude).abs() < 0.05
        }) {
            self.tab = SidebarTab::Records;
            self.selection = Some(ListSelection {
                tab: SidebarTab::Records,
                index: i,
            });
            self.active = Some(record.as_header_report());
            self.overlay_mode = OverlayMode::Wave;
            if has_coord {
                // 收到即跳：对准本条，而非「列表最新」
                self.pending_record_fly = Some((lon, lat, 7.2));
            }
        }
    }

    /// 插入火山情报；按 name+title+time 去重；新→旧。
    pub fn upsert_volcano(&mut self, notice: VolcanoNotice) {
        let dup = self.volcanoes.iter().any(|v| {
            v.name == notice.name
                && v.title == notice.title
                && v.time_text == notice.time_text
        });
        if dup {
            return;
        }
        self.volcanoes.push(notice);
        self.volcanoes.sort_by(|a, b| b.issued_ms.cmp(&a.issued_ms));
        if self.volcanoes.len() > 40 {
            self.volcanoes.truncate(40);
        }
    }

    /// 启动/空闲时跟焦最新预警；切 Tab + 刷新左上角，不锁列表选中。
    /// 返回 `(lon, lat, zoom)`。
    pub fn focus_latest_event(&mut self) -> Option<(f64, f64, f64)> {
        let idx = self.eew_list.iter().position(|e| {
            !e.is_cancel && (e.latitude.abs() > 0.01 || e.longitude.abs() > 0.01)
        })?;
        let ev = self.eew_list[idx].clone();
        let lon = ev.longitude;
        let lat = ev.latitude;
        self.active = Some(ev);
        self.overlay_mode = OverlayMode::Wave;
        self.tab = SidebarTab::Eew;
        Some((lon, lat, 7.2))
    }

    /// 无预警时取最新有坐标速报的镜头目标；不改 Tab（仅相机跟焦）。
    pub fn latest_record_camera(&self) -> Option<(f64, f64, f64, String)> {
        let idx = self.records.iter().position(|r| {
            r.latitude.abs() > 0.01 || r.longitude.abs() > 0.01
        })?;
        let rec = &self.records[idx];
        let key = format!(
            "{}|{}|{}|{:.2}",
            rec.agency.0, rec.place, rec.origin_ms, rec.magnitude
        );
        Some((rec.longitude, rec.latitude, 7.5, key))
    }

    /// 启动超时无预警时：选中最新速报并返回镜头目标。
    pub fn focus_latest_record(&mut self) -> Option<(f64, f64, f64, String)> {
        let (lon, lat, zoom, key) = self.latest_record_camera()?;
        let idx = self.records.iter().position(|r| {
            let k = format!(
                "{}|{}|{}|{:.2}",
                r.agency.0, r.place, r.origin_ms, r.magnitude
            );
            k == key
        })?;
        let rec = self.records[idx].clone();
        self.tab = SidebarTab::Records;
        self.selection = Some(ListSelection {
            tab: SidebarTab::Records,
            index: idx,
        });
        self.active = Some(rec.as_header_report());
        self.overlay_mode = OverlayMode::Wave;
        Some((lon, lat, zoom, key))
    }

    /// 选中指定台网当前震度/烈度最大的测站，并返回镜头目标。
    /// `network` 为 `kmoni` / `snet` / `kma`；`all` 表示全网。
    pub fn focus_max_station(&mut self, network: &str) -> Option<(f64, f64, f64)> {
        let prefix = format!("{network}:");
        let best = self
            .stations
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if network == "all" {
                    true
                } else {
                    s.id.starts_with(&prefix)
                }
            })
            .filter(|(_, s)| {
                s.latitude.abs() > 1e-6 || s.longitude.abs() > 1e-6
            })
            .max_by(|(_, a), (_, b)| {
                let ka = station_rank_key(a);
                let kb = station_rank_key(b);
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            });
        let (idx, s) = best?;
        let lon = s.longitude;
        let lat = s.latitude;
        let report = s.as_header_report();
        self.tab = SidebarTab::Station;
        self.selection = Some(ListSelection {
            tab: SidebarTab::Station,
            index: idx,
        });
        self.active = Some(report);
        self.overlay_mode = OverlayMode::Wave;
        Some((lon, lat, 10.0))
    }

    /// 用一帧有感测站快照替换列表（按震度降序，上限 120）
    pub fn replace_stations(&mut self, stations: Vec<StationSample>) {
        self.merge_stations("default", stations);
    }

    /// 按网络前缀合并测站帧（`kmoni:` / `snet:` / `kma:`），互不覆盖。
    pub fn merge_stations(&mut self, network: &str, mut stations: Vec<StationSample>) {
        let prefix = format!("{network}:");
        let keep_id = self
            .active
            .as_ref()
            .and_then(|e| e.event_id.strip_prefix("station:"))
            .map(|s| s.to_string());

        for s in &mut stations {
            if !s.id.starts_with(&prefix) {
                s.id = format!("{prefix}{}", s.id);
            }
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // S-Net 上游约每分钟一帧：計測未变则不刷新时间戳，避免右上角像每秒在跳
        let snet_unchanged = network == "snet" && {
            let mut old: Vec<(String, i32)> = self
                .stations
                .iter()
                .filter(|s| s.id.starts_with(&prefix))
                .map(|s| (s.id.clone(), (s.instrumental * 10.0).round() as i32))
                .collect();
            let mut neu: Vec<(String, i32)> = stations
                .iter()
                .map(|s| (s.id.clone(), (s.instrumental * 10.0).round() as i32))
                .collect();
            old.sort_by(|a, b| a.0.cmp(&b.0));
            neu.sort_by(|a, b| a.0.cmp(&b.0));
            old == neu
        };

        self.stations.retain(|s| !s.id.starts_with(&prefix));
        if !(network == "snet" && snet_unchanged) {
            self.station_updated_ms.insert(network.to_string(), now_ms);
        } else if !self.station_updated_ms.contains_key(network) {
            self.station_updated_ms.insert(network.to_string(), now_ms);
        }
        self.stations.extend(stations);
        self.stations.sort_by(|a, b| {
            b.instrumental
                .partial_cmp(&a.instrumental)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.intensity_level.cmp(&a.intensity_level))
                .then_with(|| a.id.cmp(&b.id))
        });
        // 全网测站上图与侧栏；上限放宽以免截掉低計測值点
        if self.stations.len() > 2500 {
            self.stations.truncate(2500);
        }

        // 有传播中预警时，测站帧不得把 active 抢回测站（否则左上角预警条/跟焦丢失）
        let has_live_eew = self.eew_list.iter().any(|e| eew_wave_active(e, now_ms));
        if let Some(id) = keep_id {
            if !has_live_eew {
                if let Some((i, s)) = self
                    .stations
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.id == id)
                {
                    self.active = Some(s.as_header_report());
                    self.overlay_mode = OverlayMode::MarkerOnly;
                    self.selection = Some(ListSelection {
                        tab: SidebarTab::Station,
                        index: i,
                    });
                }
            }
        }
    }
}

/// 可作为地图波圈绘制的预警（非测站/速报头、未取消、有坐标）。
/// 台网内「最大」排序键：烈度用等级，震度用計測值。
fn station_rank_key(s: &StationSample) -> f64 {
    match s.intensity_kind {
        IntensityKind::CnIntensity => s.intensity_level as f64,
        IntensityKind::JmaShindo => s.instrumental,
    }
}

pub fn is_wave_candidate(e: &EewReport) -> bool {
    !e.is_cancel
        && !e.event_id.starts_with("station:")
        && !e.event_id.starts_with("record:")
        && (e.latitude.abs() > 0.01 || e.longitude.abs() > 0.01)
}

/// 与 jian_travel::max_s_wave_spread_km 对齐的本地副本（core 不依赖 travel）。
fn approx_max_s_km(magnitude: f64) -> f64 {
    if magnitude <= 2.0 {
        200.0
    } else if magnitude <= 2.5 {
        350.0
    } else if magnitude <= 3.0 {
        500.0
    } else if magnitude <= 3.5 {
        700.0
    } else if magnitude <= 4.0 {
        900.0
    } else if magnitude <= 4.5 {
        1200.0
    } else if magnitude <= 5.0 {
        1600.0
    } else if magnitude <= 5.5 {
        2100.0
    } else if magnitude <= 6.0 {
        2600.0
    } else if magnitude <= 6.5 {
        3400.0
    } else if magnitude <= 7.0 {
        4200.0
    } else if magnitude <= 7.5 {
        5200.0
    } else if magnitude <= 8.0 {
        6200.0
    } else {
        7500.0
    }
}

/// 按自定义表或内置表取 S 波最大传播距离（km）。
/// `custom`：(min_mag, max_mag, max_km)；命中区间则用该档，否则回退内置表。
pub fn max_s_spread_km(magnitude: f64, custom: &[(f64, f64, f64)]) -> f64 {
    if custom.is_empty() {
        return approx_max_s_km(magnitude);
    }
    for &(lo, hi, km) in custom {
        let a = lo.min(hi);
        let b = lo.max(hi);
        if magnitude + 1e-9 >= a && magnitude <= b + 1e-9 {
            return km;
        }
    }
    approx_max_s_km(magnitude)
}

/// S 波是否仍在传播（地表等效约 5 km/s，与走时兜底一致；走完即视为结束）。
/// 与 GlobalQuake 一致：不按「最终报 + N 分钟」人为延长，波走完即结束。
pub fn eew_wave_active(e: &EewReport, now_ms: i64) -> bool {
    eew_wave_active_spread(e, now_ms, &[])
}

/// 同 [`eew_wave_active`]，可传入自定义震级区间→传播距离表。
pub fn eew_wave_active_spread(e: &EewReport, now_ms: i64, custom: &[(f64, f64, f64)]) -> bool {
    if !is_wave_candidate(e) {
        return false;
    }
    if e.origin_ms <= 0 {
        return true; // 演示样本
    }
    let age_ms = now_ms - e.origin_ms;
    if age_ms < -60_000 {
        return false;
    }
    let elapsed_s = (age_ms as f64 / 1000.0).max(0.0);
    let max_km = max_s_spread_km(e.magnitude, custom);
    // 无走时表时的近似：对齐 eewcn SWAVE_SPEED=4（勿用 5，否则左上条比波圈更早消失）
    elapsed_s * 4.0 < max_km * 1.05
}

/// 仍在传播中的多震：PS 走完即排除（不按 30 分钟挂着）。`max_age_ms` 仅兜底。
pub fn live_wave_eews(list: &[EewReport], now_ms: i64, _max_age_ms: i64) -> Vec<&EewReport> {
    live_wave_eews_spread(list, now_ms, &[])
}

pub fn live_wave_eews_spread<'a>(
    list: &'a [EewReport],
    now_ms: i64,
    spread: &[(f64, f64, f64)],
) -> Vec<&'a EewReport> {
    list.iter()
        .filter(|e| eew_wave_active_spread(e, now_ms, spread))
        .collect()
}

/// 地图叉标：与波圈相同，PS 消失即去掉（GQ 多震不另留 30 分钟）。
pub fn eew_map_markers(list: &[EewReport], now_ms: i64) -> Vec<&EewReport> {
    live_wave_eews(list, now_ms, 0)
}

pub fn eew_map_markers_spread<'a>(
    list: &'a [EewReport],
    now_ms: i64,
    spread: &[(f64, f64, f64)],
) -> Vec<&'a EewReport> {
    live_wave_eews_spread(list, now_ms, spread)
}

/// 预警侧栏：每个侧栏源槽只显示最新一条（跨 API 镜像已去重）；波走完仍留这条供回看。
/// 国网 CEA 与省级 cea-pr 分槽，避免地方局挤掉 `[CN]`。
/// 地图多震仍用 live_wave_eews，与列表无关。
pub fn eew_sidebar_entries(list: &[EewReport]) -> Vec<(usize, &EewReport)> {
    latest_eew_per_source(list)
        .into_iter()
        .filter_map(|e| {
            list.iter()
                .position(|x| x.agency.0 == e.agency.0 && x.event_id == e.event_id)
                .map(|i| (i, e))
        })
        .collect()
}

/// 侧栏分组键：同族镜像合并；国网 CEA（`[CN]`）与省级 cea-pr 分槽。
/// 各省 cea-pr（宁夏/甘肃/云南…）共用一个 `cea-pr` 槽，避免同震多省残留。
pub fn eew_sidebar_group_key(agency: &str) -> String {
    if cea_pr_province(agency).is_some() {
        return "cea-pr".into();
    }
    agency_family(agency).as_filter_key().to_string()
}

/// 每个侧栏源槽（如 JMA，含 Jian/Wolfx 镜像；CEA 国网与各省分槽）只保留发震最新的一条。
pub fn latest_eew_per_source(list: &[EewReport]) -> Vec<&EewReport> {
    let mut best: Vec<&EewReport> = Vec::new();
    for e in list {
        if e.is_cancel
            || e.event_id.starts_with("station:")
            || e.event_id.starts_with("record:")
            || (e.latitude.abs() <= 0.01 && e.longitude.abs() <= 0.01)
        {
            continue;
        }
        let key = eew_sidebar_group_key(&e.agency.0);
        if let Some(pos) = best
            .iter()
            .position(|b| eew_sidebar_group_key(&b.agency.0) == key)
        {
            let cur = best[pos];
            let newer = recency_ms(e.origin_ms) > recency_ms(cur.origin_ms)
                || (e.origin_ms == cur.origin_ms && prefer_eew_report(cur, e));
            if newer {
                best[pos] = e;
            }
        } else {
            best.push(e);
        }
    }
    best.sort_by(|a, b| {
        recency_ms(b.origin_ms)
            .cmp(&recency_ms(a.origin_ms))
            .then(b.serial.cmp(&a.serial))
    });
    best
}

/// 速报内容是否实质相同（列表重放判定）。
fn records_content_equivalent(a: &EqRecord, b: &EqRecord) -> bool {
    a.is_auto == b.is_auto
        && (a.magnitude - b.magnitude).abs() < 0.05
        && a.intensity_level == b.intensity_level
        && a.place == b.place
        && (a.depth_km - b.depth_km).abs() < 0.5
        && (a.latitude - b.latitude).abs() < 0.02
        && (a.longitude - b.longitude).abs() < 0.02
}

/// 近时速报才触发 TTS（避免冷启动/定期 list 朗读历史条目）。
fn record_fresh_for_tts(origin_ms: i64) -> bool {
    if origin_ms <= 0 {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now.saturating_sub(origin_ms) < 45 * 60 * 1000
}

/// 跨 API 是否同一 EEW 事件（同机构族 + 同 EventID，或发震时刻/震中接近）。
fn same_record_event(a: &EqRecord, b: &EqRecord) -> bool {
    if agency_family(&a.agency.0) != agency_family(&b.agency.0) {
        return false;
    }
    if a.origin_ms > 0 && b.origin_ms > 0 {
        if (a.origin_ms - b.origin_ms).abs() > 120_000 {
            return false;
        }
    } else if a.time_text != b.time_text {
        return false;
    }
    let dlat = (a.latitude - b.latitude).abs();
    let dlon = (a.longitude - b.longitude).abs();
    let dmag = (a.magnitude - b.magnitude).abs();
    dlat <= 0.35 && dlon <= 0.35 && dmag <= 0.6
}

pub fn same_eew_event(a: &EewReport, b: &EewReport) -> bool {
    // 国网 CEA 与各省 cea-pr 分源；同槽内 Jian/Wolfx 镜像仍合并
    if eew_sidebar_group_key(&a.agency.0) != eew_sidebar_group_key(&b.agency.0) {
        return false;
    }
    let id_a = a.event_id.trim();
    let id_b = b.event_id.trim();
    let id_usable = |id: &str| {
        !id.is_empty()
            && id != "—"
            && id != "wolfx-unknown"
            && !id.starts_with("anon:")
            && !id.starts_with("station:")
            && !id.starts_with("record:")
    };
    if id_usable(id_a) && id_usable(id_b) && id_a == id_b {
        return true;
    }
    if a.origin_ms > 0 && b.origin_ms > 0 && (a.origin_ms - b.origin_ms).abs() <= 180_000 {
        let dlat = (a.latitude - b.latitude).abs();
        let dlon = (a.longitude - b.longitude).abs();
        let dmag = (a.magnitude - b.magnitude).abs();
        if dlat <= 0.5 && dlon <= 0.5 && dmag <= 0.8 {
            return true;
        }
    }
    false
}

/// `new` 是否应覆盖 `old`：报数优先，其次镜像源优先级。
fn prefer_eew_report(old: &EewReport, new: &EewReport) -> bool {
    if new.serial != old.serial {
        return new.serial > old.serial;
    }
    eew_feed_priority(&new.agency.0) >= eew_feed_priority(&old.agency.0)
}

fn recency_ms(origin_ms: i64) -> i64 {
    if origin_ms <= 0 {
        i64::MIN / 4
    } else {
        origin_ms
    }
}

/// 列表回看：每 agency 保留最新非取消一条（对齐 eewcn）；其余按龄/条数裁剪。
fn prune_eew_list(list: &mut Vec<EewReport>, max_age_ms: i64, max_len: usize) {
    if list.is_empty() {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // 每源最新（非取消、有坐标）必须保留，否则冷启动历史会被 6h 窗清空
    let mut keep_ids: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    // 每侧栏源槽最新一条（国网 CEA / 各省 cea-pr 分槽；跨 API 镜像只留一条）
    let mut best_by_family: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, e) in list.iter().enumerate() {
        if e.is_cancel
            || e.event_id.starts_with("station:")
            || e.event_id.starts_with("record:")
            || (e.latitude.abs() <= 0.01 && e.longitude.abs() <= 0.01)
        {
            continue;
        }
        let fam = eew_sidebar_group_key(&e.agency.0);
        match best_by_family.get(&fam) {
            None => {
                best_by_family.insert(fam, i);
            }
            Some(&j) => {
                let cur = &list[j];
                let newer = recency_ms(e.origin_ms) > recency_ms(cur.origin_ms)
                    || (e.origin_ms == cur.origin_ms && prefer_eew_report(cur, e));
                if newer {
                    best_by_family.insert(fam, i);
                }
            }
        }
    }
    for &i in best_by_family.values() {
        keep_ids.insert((list[i].agency.0.clone(), list[i].event_id.clone()));
    }

    list.retain(|e| {
        if e.event_id.starts_with("station:") || e.event_id.starts_with("record:") {
            return false;
        }
        if keep_ids.contains(&(e.agency.0.clone(), e.event_id.clone())) {
            return true;
        }
        if e.origin_ms <= 0 {
            return true;
        }
        let age = now_ms - e.origin_ms;
        age >= -60_000 && age <= max_age_ms
    });
    if list.len() > max_len {
        // 先保住每源最新，再按新→旧截断
        let protected: std::collections::HashSet<(String, String)> = keep_ids;
        sort_eew_newest_first(list);
        let mut out = Vec::with_capacity(max_len);
        for e in list.drain(..) {
            let key = (e.agency.0.clone(), e.event_id.clone());
            if protected.contains(&key) || out.len() < max_len {
                if out.len() < max_len || protected.contains(&key) {
                    out.push(e);
                }
            }
        }
        // 若 protected 把长度顶破，再按新→旧截到 max_len（仍尽量留 protected）
        if out.len() > max_len {
            let mut keep_p: Vec<EewReport> = Vec::new();
            let mut rest: Vec<EewReport> = Vec::new();
            for e in out {
                let key = (e.agency.0.clone(), e.event_id.clone());
                if protected.contains(&key) {
                    keep_p.push(e);
                } else {
                    rest.push(e);
                }
            }
            sort_eew_newest_first(&mut rest);
            let room = max_len.saturating_sub(keep_p.len());
            rest.truncate(room);
            out = keep_p;
            out.extend(rest);
            sort_eew_newest_first(&mut out);
        }
        *list = out;
    }
}

fn sort_eew_newest_first(list: &mut [EewReport]) {
    list.sort_by(|a, b| {
        recency_ms(b.origin_ms)
            .cmp(&recency_ms(a.origin_ms))
            .then(b.serial.cmp(&a.serial))
    });
}

fn sort_records_newest_first(list: &mut [EqRecord]) {
    list.sort_by(|a, b| recency_ms(b.origin_ms).cmp(&recency_ms(a.origin_ms)));
}

#[cfg(test)]
mod eew_dedup_tests {
    use super::*;

    fn sample(agency: &str, event_id: &str, serial: u32) -> EewReport {
        EewReport {
            agency: AgencyId(agency.into()),
            event_id: event_id.into(),
            serial,
            place: "宫古岛近海".into(),
            magnitude: 4.6,
            depth_km: 10.0,
            latitude: 24.8,
            longitude: 125.3,
            origin_ms: 1_728_650_000_000,
            max_intensity_text: "2".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 2,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        }
    }

    #[test]
    fn upsert_merges_jian_and_wolfx_jma() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        snap.active = None;
        snap.upsert_eew(sample("wolfx-jma-eew", "20261011201300", 6));
        snap.upsert_eew(sample("jma-eew", "20261011201300", 6));
        assert_eq!(snap.eew_list.len(), 1);
        assert_eq!(snap.eew_list[0].agency.0, "jma-eew");
        let side = eew_sidebar_entries(&snap.eew_list);
        assert_eq!(side.len(), 1);
    }

    #[test]
    fn sidebar_keeps_national_cea_and_cea_pr() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        let mut cea = sample("cea", "cn-1", 2);
        cea.place = "缅甸".into();
        cea.latitude = 22.0;
        cea.longitude = 98.5;
        cea.origin_ms = 1_728_650_000_000;
        let mut pr = sample("cea-pr:云南", "pr-1", 1);
        pr.place = "云南某地".into();
        pr.latitude = 25.0;
        pr.longitude = 102.0;
        pr.origin_ms = 1_728_651_000_000;
        snap.upsert_eew(cea);
        snap.upsert_eew(pr);
        let side = eew_sidebar_entries(&snap.eew_list);
        assert_eq!(side.len(), 2);
        let agencies: Vec<_> = side.iter().map(|(_, e)| e.agency.0.as_str()).collect();
        assert!(agencies.iter().any(|a| *a == "cea"));
        assert!(agencies.iter().any(|a| *a == "cea-pr:云南"));
    }

    #[test]
    fn sidebar_merges_all_cea_pr_provinces_to_one_slot() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        let mut nx = sample("cea-pr:宁夏", "nx-1", 1);
        nx.place = "宁夏隆德县".into();
        nx.latitude = 35.6;
        nx.longitude = 106.1;
        nx.origin_ms = 1_728_650_000_000;
        let mut gs = sample("cea-pr:甘肃", "gs-1", 2);
        gs.place = "宁夏固原市原州区".into();
        gs.latitude = 36.0;
        gs.longitude = 106.2;
        gs.origin_ms = 1_728_650_000_000;
        let mut cn = sample("cea", "cn-1", 2);
        cn.place = "宁夏固原市原州区".into();
        cn.latitude = 36.0;
        cn.longitude = 106.2;
        cn.origin_ms = 1_728_650_000_000;
        snap.upsert_eew(nx);
        snap.upsert_eew(gs);
        snap.upsert_eew(cn);
        let side = eew_sidebar_entries(&snap.eew_list);
        assert_eq!(side.len(), 2);
        let keys: Vec<_> = side
            .iter()
            .map(|(_, e)| eew_sidebar_group_key(&e.agency.0))
            .collect();
        assert!(keys.iter().any(|k| k == "cea"));
        assert!(keys.iter().any(|k| k == "cea-pr"));
        // 省级槽保留更新报（甘肃 #2）
        let pr = side
            .iter()
            .find(|(_, e)| eew_sidebar_group_key(&e.agency.0) == "cea-pr")
            .unwrap()
            .1;
        assert_eq!(pr.agency.0, "cea-pr:甘肃");
        assert_eq!(pr.serial, 2);
    }

    #[test]
    fn wolfx_cenc_eew_merges_into_cn_slot_prefer_jian() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        snap.upsert_eew(sample("wolfx-cea", "cn-evt", 3));
        snap.upsert_eew(sample("cea", "cn-evt", 3));
        assert_eq!(snap.eew_list.len(), 1);
        assert_eq!(snap.eew_list[0].agency.0, "cea");
        assert_eq!(eew_sidebar_group_key("wolfx-cea"), "cea");
        assert_eq!(agency_bracket("wolfx-cea"), "CN");
    }

    #[test]
    fn stale_record_list_replay_skips_tts() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        snap.records.clear();
        snap.pending_tts_record = None;
        let old = EqRecord {
            agency: AgencyId("cenc".into()),
            place: "测试地点".into(),
            time_text: "2020-01-01".into(),
            magnitude: 4.0,
            intensity_text: "Ⅲ".into(),
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: 3,
            latitude: 30.0,
            longitude: 104.0,
            depth_km: 10.0,
            origin_ms: 1_577_836_800_000, // 2020
            is_auto: false,
        };
        snap.upsert_record(old.clone());
        assert!(snap.pending_tts_record.is_none());
        // 同内容重放仍不应 TTS
        snap.upsert_record(old);
        assert!(snap.pending_tts_record.is_none());
    }

    #[test]
    fn higher_serial_from_wolfx_updates_jian_slot() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        snap.active = None;
        snap.upsert_eew(sample("jma-eew", "20261011201300", 5));
        snap.upsert_eew(sample("wolfx-jma-eew", "20261011201300", 6));
        assert_eq!(snap.eew_list.len(), 1);
        assert_eq!(snap.eew_list[0].serial, 6);
    }

    #[test]
    fn live_eew_blocks_record_from_stealing_active() {
        let mut snap = AppSnapshot::offline_sample();
        snap.eew_list.clear();
        snap.records.clear();
        snap.active = None;
        let mut eew = sample("jma-eew", "20261011201300", 3);
        eew.origin_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        snap.upsert_eew(eew.clone());
        assert_eq!(snap.tab, SidebarTab::Eew);

        // active 被清掉时，旧逻辑会让速报抢走；传播中预警必须挡住
        snap.active = None;
        snap.upsert_record(EqRecord {
            agency: AgencyId("p2pquake".into()),
            place: "宫古岛近海".into(),
            time_text: "2026-10-12 12:00".into(),
            magnitude: 4.6,
            intensity_text: "2".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 2,
            latitude: 24.8,
            longitude: 125.3,
            depth_km: 10.0,
            origin_ms: eew.origin_ms,
            is_auto: false,
        });
        assert!(snap.records.iter().any(|r| r.place == "宫古岛近海"));
        assert!(snap.pending_tts_record.is_some());
        assert!(snap.active.is_none());
        assert_eq!(snap.tab, SidebarTab::Eew);
        assert!(snap.pending_record_fly.is_none());
    }
}
