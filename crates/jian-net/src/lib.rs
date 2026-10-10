//! 三源网络：Jian / Wolfx / P2PQuake → 归一化事件总线。

pub mod adapter;
pub mod bus;
pub mod auth;
pub mod place;
pub mod sources;

pub use auth::{
    exchange_access_token, exchange_login_key, format_expire_date, verify_refresh_token, TokenOk,
};
pub mod time_parse;

use jian_config::{FiltersConfig, SourcesConfig};
use jian_core::agency_family;
use tokio::task::JoinHandle;
use tracing::info;

pub use bus::{channel, NetEvent, NetRx, NetTx};
pub use place::{fix_place, load_from_dir as load_place_fix};

/// 三源任务句柄：丢弃或 `shutdown` 会中止连接循环，便于设置页热切换。
pub struct NetHub {
    rx: NetRx,
    tasks: Vec<JoinHandle<()>>,
}

impl NetHub {
    pub fn try_recv(&mut self) -> Result<NetEvent, tokio::sync::mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }

    pub fn shutdown(&mut self) {
        for t in self.tasks.drain(..) {
            t.abort();
        }
    }
}

impl Drop for NetHub {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Jian,
    Wolfx,
    P2pQuake,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jian => "jian",
            Self::Wolfx => "wolfx",
            Self::P2pQuake => "p2pquake",
        }
    }
}

/// 在已有 tokio runtime 内启动情报源与测站任务。
pub fn spawn_hub(sources: &SourcesConfig) -> NetHub {
    spawn_hub_with_assets(sources, None)
}

/// `assets_stations` 指向含 `nied_point.csv` 的目录（原版 kmoni/lmoni）。
pub fn spawn_hub_with_assets(
    sources: &SourcesConfig,
    assets_stations: Option<std::path::PathBuf>,
) -> NetHub {
    let (tx, rx) = channel();
    info!(
        jian = sources.jian.enabled,
        wolfx = sources.wolfx.enabled,
        p2p = sources.p2pquake.enabled,
        jma_st = sources.stations.jma_enabled,
        snet = sources.stations.snet_enabled,
        kma = sources.stations.kma_enabled,
        "jian-net: spawning source tasks"
    );
    let mut tasks = Vec::new();
    // 测站由 stations 模块统一管理；jian 侧不再重复拉 /kmoni
    let mut jian_cfg = sources.jian.clone();
    jian_cfg.station_enabled = false;
    tasks.extend(sources::jian::spawn(tx.clone(), jian_cfg));
    tasks.extend(sources::wolfx::spawn(tx.clone(), sources.wolfx.clone()));
    tasks.extend(sources::p2pquake::spawn(tx.clone(), sources.p2pquake.clone()));
    tasks.extend(sources::stations::spawn(
        tx,
        sources.stations.clone(),
        assets_stations,
    ));
    NetHub { rx, tasks }
}

/// 将 `NetEvent` 合并进 `AppSnapshot`（供 UI 帧循环调用）。
pub fn apply_event(snap: &mut jian_core::AppSnapshot, ev: NetEvent) {
    apply_event_filtered(snap, ev, None);
}

/// 带 RhythmQuake 风格机构开关 / 震级阈值过滤。
pub fn apply_event_filtered(
    snap: &mut jian_core::AppSnapshot,
    ev: NetEvent,
    filters: Option<&FiltersConfig>,
) {
    match ev {
        NetEvent::Health { source, status } => snap.set_health(source, status),
        NetEvent::Eew(e) => {
            if let Some(f) = filters {
                let key = agency_family(&e.agency.0).as_filter_key();
                if !f.eew_agency_on(key) {
                    return;
                }
            }
            snap.upsert_eew(e);
        }
        NetEvent::Record(r) => {
            if let Some(f) = filters {
                if !record_allowed(f, &r.agency.0, r.magnitude, &r.place) {
                    return;
                }
            }
            snap.upsert_record(r);
        }
        NetEvent::Stations { network, list } => snap.merge_stations(network, list),
        NetEvent::Tsunami(t) => {
            if t.cancelled || t.grade.eq_ignore_ascii_case("Cancel") {
                snap.set_tsunami(None);
            } else {
                snap.set_tsunami(Some(t));
            }
        }
    }
}

fn record_allowed(f: &FiltersConfig, agency: &str, magnitude: f64, place: &str) -> bool {
    let key = agency_family(agency).as_filter_key();
    let thr = f.record_mag_threshold(key);
    if thr < 0.0 {
        return false;
    }
    if thr == 0.0 {
        return true;
    }
    if magnitude + 1e-9 >= thr {
        return true;
    }
    f.place_whitelist_hit(place)
}
