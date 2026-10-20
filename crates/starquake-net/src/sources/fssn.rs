//! FSSN 地震信息（`wss://fssn-api.fanstudio.tech/fssn`）。
//!
//! 连接后发送 `fssnlist` 拉取已有事件；之后收 `initial` / `update`。

use starquake_config::FssnSourceConfig;
use tokio::task::JoinHandle;
use tracing::info;

use crate::adapter::ingest_fssn_frame;
use crate::bus::NetTx;
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

pub fn spawn(tx: NetTx, cfg: FssnSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        return Vec::new();
    }
    let url = cfg.ws_url.clone();
    info!(%url, "fssn: spawn ws");
    vec![tokio::spawn(async move {
        run_ws_loop(
            tx,
            WsLoopOpts {
                source: "fssn",
                url,
                resolve_url: None,
                on_open_texts: vec!["fssnlist".into()],
                periodic_texts: vec!["ping".into()],
                periodic_secs: 20,
                backoff_max_secs: 60,
                stale_secs: 60,
            },
            move |tx, text| {
                ingest_fssn_frame(tx, text);
            },
        )
        .await;
    })]
}
