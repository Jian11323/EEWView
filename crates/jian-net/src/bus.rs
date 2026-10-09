//! 网络事件总线（tokio mpsc）。

use jian_core::{EewReport, EqRecord, HealthStatus, StationSample};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum NetEvent {
    Health {
        source: &'static str,
        status: HealthStatus,
    },
    Eew(EewReport),
    Record(EqRecord),
    #[allow(dead_code)]
    Station(StationSample),
}

pub type NetTx = mpsc::UnboundedSender<NetEvent>;
pub type NetRx = mpsc::UnboundedReceiver<NetEvent>;

pub fn channel() -> (NetTx, NetRx) {
    mpsc::unbounded_channel()
}

pub fn emit_health(tx: &NetTx, source: &'static str, status: HealthStatus) {
    let _ = tx.send(NetEvent::Health { source, status });
}
