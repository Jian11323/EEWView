//! 网络事件总线（tokio mpsc）。

use jian_core::{EewReport, EqRecord, HealthStatus, StationSample, TsunamiInfo};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum NetEvent {
    Health {
        source: &'static str,
        status: HealthStatus,
    },
    Eew(EewReport),
    Record(EqRecord),
    /// 有感测站一帧快照；`network` 为 `kmoni` / `snet` / `kma` 等，用于多网合并
    Stations {
        network: &'static str,
        list: Vec<StationSample>,
    },
    /// JMA 海啸情报（P2PQuake code 552 等）
    Tsunami(TsunamiInfo),
}

pub type NetTx = mpsc::UnboundedSender<NetEvent>;
pub type NetRx = mpsc::UnboundedReceiver<NetEvent>;

pub fn channel() -> (NetTx, NetRx) {
    mpsc::unbounded_channel()
}

pub fn emit_health(tx: &NetTx, source: &'static str, status: HealthStatus) {
    let _ = tx.send(NetEvent::Health { source, status });
}
