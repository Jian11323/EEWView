//! 三源网络：Jian / Wolfx / P2PQuake → 归一化事件总线。

pub mod adapter;
pub mod bus;
pub mod sources;
pub mod time_parse;

use jian_config::SourcesConfig;
use tracing::info;

pub use bus::{channel, NetEvent, NetRx, NetTx};

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

/// 在已有 tokio runtime 内启动三源任务；返回事件接收端。
pub fn spawn_hub(sources: &SourcesConfig) -> NetRx {
    let (tx, rx) = channel();
    info!(
        jian = sources.jian.enabled,
        wolfx = sources.wolfx.enabled,
        p2p = sources.p2pquake.enabled,
        "jian-net: spawning source tasks"
    );
    sources::jian::spawn(tx.clone(), sources.jian.clone());
    sources::wolfx::spawn(tx.clone(), sources.wolfx.clone());
    sources::p2pquake::spawn(tx, sources.p2pquake.clone());
    rx
}

/// 将 `NetEvent` 合并进 `AppSnapshot`（供 UI 帧循环调用）。
pub fn apply_event(snap: &mut jian_core::AppSnapshot, ev: NetEvent) {
    match ev {
        NetEvent::Health { source, status } => snap.set_health(source, status),
        NetEvent::Eew(e) => snap.upsert_eew(e),
        NetEvent::Record(r) => snap.upsert_record(r),
        NetEvent::Station(s) => {
            // P5：测站；骨架仅追加不去重
            if !snap.stations.iter().any(|x| x.id == s.id) {
                snap.stations.push(s);
            }
        }
    }
}
