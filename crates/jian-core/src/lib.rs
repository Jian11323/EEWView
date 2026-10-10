//! 领域模型与官方色标。

pub mod agency;
pub mod estimate;
pub mod intensity;
pub mod palette;

use serde::{Deserialize, Serialize};

pub use agency::{
    agency_bracket, agency_family, cea_pr_agency_id, place_with_agency, AgencyFamily,
    EEW_FILTER_FAMILIES, RECORD_FILTER_FAMILIES,
};
pub use estimate::{
    calc_csis, calc_jma_instrumental, estimate_at_site, estimate_epicentral, fill_intensity,
    hypocentral_km,
};
pub use intensity::{
    cn_intensity_level, cn_intensity_text, intensity_kind_for_agency, jma_from_instrumental,
    jma_shindo_level, jma_shindo_text, prefer_intensity_scale,
};

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
    pub health_jian: HealthStatus,
    pub health_wolfx: HealthStatus,
    pub health_p2p: HealthStatus,
    pub tab: SidebarTab,
    pub selection: Option<ListSelection>,
    /// 图例 P-S：是否绘制走时波圈
    pub show_wave_rings: bool,
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
        };

        Self {
            active: Some(eew_a.clone()),
            overlay_mode: OverlayMode::Wave,
            records: vec![
                EqRecord {
                    agency: AgencyId("cenc".into()),
                    place: "四川阿坝州汶川县".into(),
                    time_text: "10-09 12:01".into(),
                    magnitude: 4.2,
                    intensity_text: "Ⅴ".into(),
                    intensity_kind: IntensityKind::CnIntensity,
                    intensity_level: 5,
                    latitude: 31.5,
                    longitude: 103.6,
                    depth_km: 12.0,
                    origin_ms: 0,
                },
                EqRecord {
                    agency: AgencyId("jma".into()),
                    place: "千叶县西北部".into(),
                    time_text: "10-09 11:58".into(),
                    magnitude: 5.4,
                    intensity_text: "5弱".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 5,
                    latitude: 35.7,
                    longitude: 140.1,
                    depth_km: 40.0,
                    origin_ms: 0,
                },
                EqRecord {
                    agency: AgencyId("jma".into()),
                    place: "福岛县中通".into(),
                    time_text: "10-09 09:12".into(),
                    magnitude: 4.6,
                    intensity_text: "4".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 4,
                    latitude: 37.4,
                    longitude: 140.4,
                    depth_km: 10.0,
                    origin_ms: 0,
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
                },
                StationSample {
                    id: "OSAKA".into(),
                    name: "大阪".into(),
                    latitude: 34.69,
                    longitude: 135.50,
                    intensity_text: "2".into(),
                    intensity_kind: IntensityKind::JmaShindo,
                    intensity_level: 2,
                },
                StationSample {
                    id: "CD01".into(),
                    name: "成都".into(),
                    latitude: 30.67,
                    longitude: 104.06,
                    intensity_text: "Ⅳ".into(),
                    intensity_kind: IntensityKind::CnIntensity,
                    intensity_level: 4,
                },
            ],
            countdown_s: Some(28),
            health_jian: HealthStatus::Normal,
            health_wolfx: HealthStatus::Normal,
            health_p2p: HealthStatus::Fluctuating,
            tab: SidebarTab::Eew,
            selection: Some(ListSelection {
                tab: SidebarTab::Eew,
                index: 0,
            }),
            show_wave_rings: true,
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
                Some((lon, lat, 6.0))
            }
            SidebarTab::Records => {
                let r = self.records.get(index)?;
                let lon = r.longitude;
                let lat = r.latitude;
                self.active = Some(r.as_header_report());
                self.overlay_mode = OverlayMode::Wave;
                Some((lon, lat, 6.5))
            }
            SidebarTab::Station => {
                let s = self.stations.get(index)?;
                let lon = s.longitude;
                let lat = s.latitude;
                self.active = Some(s.as_header_report());
                self.overlay_mode = OverlayMode::MarkerOnly;
                Some((lon, lat, 8.0))
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
            health_jian: HealthStatus::Abnormal,
            health_wolfx: HealthStatus::Abnormal,
            health_p2p: HealthStatus::Abnormal,
            tab: SidebarTab::Eew,
            selection: None,
            show_wave_rings: true,
        }
    }

    pub fn set_health(&mut self, source: &str, status: HealthStatus) {
        match source {
            "jian" => self.health_jian = status,
            // 测站 /kmoni 计入 Jian：无 /all 令牌时仍可显示「正常」
            "kmoni" => {
                if status == HealthStatus::Normal {
                    self.health_jian = HealthStatus::Normal;
                } else if self.health_jian != HealthStatus::Normal {
                    self.health_jian = status;
                }
            }
            "wolfx" => self.health_wolfx = status,
            "p2pquake" | "p2p" => self.health_p2p = status,
            _ => {}
        }
    }

    /// 插入/更新 EEW；同 agency+event_id 保留更高 serial；列表按发震时刻新→旧
    pub fn upsert_eew(&mut self, report: EewReport) {
        let key = (report.agency.0.clone(), report.event_id.clone());
        if let Some(pos) = self
            .eew_list
            .iter()
            .position(|e| e.agency.0 == key.0 && e.event_id == key.1)
        {
            if report.serial >= self.eew_list[pos].serial {
                self.eew_list[pos] = report.clone();
            } else {
                return;
            }
        } else {
            self.eew_list.push(report.clone());
        }
        sort_eew_newest_first(&mut self.eew_list);
        if self.eew_list.len() > 50 {
            self.eew_list.truncate(50);
        }
        self.rebind_eew_selection(&key);

        // 无选中，或选中正是该 EEW → 刷新 Header；用户点了 Records/Station 则不抢焦点
        let refresh_active = match &self.selection {
            None => true,
            Some(sel) if sel.tab == SidebarTab::Eew => self
                .eew_list
                .get(sel.index)
                .map(|e| e.agency.0 == key.0 && e.event_id == key.1)
                .unwrap_or(true),
            Some(_) => false,
        };
        if refresh_active || self.active.is_none() {
            if let Some(pos) = self
                .eew_list
                .iter()
                .position(|e| e.agency.0 == key.0 && e.event_id == key.1)
            {
                self.active = Some(self.eew_list[pos].clone());
                self.overlay_mode = OverlayMode::Wave;
                if self.selection.is_none() {
                    self.selection = Some(ListSelection {
                        tab: SidebarTab::Eew,
                        index: pos,
                    });
                    self.tab = SidebarTab::Eew;
                }
            }
        }
    }

    fn rebind_eew_selection(&mut self, key: &(String, String)) {
        if let Some(sel) = self.selection {
            if sel.tab == SidebarTab::Eew {
                if let Some(i) = self
                    .eew_list
                    .iter()
                    .position(|e| e.agency.0 == key.0 && e.event_id == key.1)
                {
                    self.selection = Some(ListSelection {
                        tab: SidebarTab::Eew,
                        index: i,
                    });
                }
            }
        }
    }

    /// 插入速报；按 agency+place+time 粗去重；列表按发震时刻新→旧
    pub fn upsert_record(&mut self, record: EqRecord) {
        let dup = self.records.iter().any(|r| {
            r.agency == record.agency
                && r.place == record.place
                && r.time_text == record.time_text
                && (r.magnitude - record.magnitude).abs() < 0.05
        });
        if dup {
            return;
        }
        self.records.push(record);
        sort_records_newest_first(&mut self.records);
        if self.records.len() > 80 {
            self.records.truncate(80);
        }
        if let Some(sel) = self.selection {
            if sel.tab == SidebarTab::Records {
                // 新项插入后下标可能变化；保持选中项身份
                if let Some(cur) = self.active.as_ref() {
                    if let Some(i) = self.records.iter().position(|r| {
                        r.agency.0 == cur.agency.0 && r.place == cur.place && r.origin_ms == cur.origin_ms
                    }) {
                        self.selection = Some(ListSelection {
                            tab: SidebarTab::Records,
                            index: i,
                        });
                    }
                }
            }
        }
    }

    /// 用一帧有感测站快照替换列表（按震度降序，上限 120）
    pub fn replace_stations(&mut self, mut stations: Vec<StationSample>) {
        let keep_id = self
            .active
            .as_ref()
            .and_then(|e| e.event_id.strip_prefix("station:"))
            .map(|s| s.to_string());

        stations.sort_by(|a, b| {
            b.intensity_level
                .cmp(&a.intensity_level)
                .then_with(|| a.id.cmp(&b.id))
        });
        if stations.len() > 120 {
            stations.truncate(120);
        }
        self.stations = stations;

        if let Some(id) = keep_id {
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

fn recency_ms(origin_ms: i64) -> i64 {
    if origin_ms <= 0 {
        i64::MIN / 4
    } else {
        origin_ms
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
