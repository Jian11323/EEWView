//! Jian Project API（`api.sismotide.top`）。

use jian_config::JianSourceConfig;
use jian_core::HealthStatus;
use tracing::{info, warn};

use crate::adapter::ingest_jian_frame;
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

pub fn spawn(tx: NetTx, cfg: JianSourceConfig) {
    if !cfg.enabled {
        emit_health(&tx, "jian", HealthStatus::Abnormal);
        return;
    }
    let token = cfg.access_token.clone().filter(|t| !t.trim().is_empty());
    let Some(token) = token else {
        warn!("jian: 未配置 JIAN_ACCESS_TOKEN，跳过连接（健康态=异常）");
        emit_health(&tx, "jian", HealthStatus::Abnormal);
        return;
    };

    let mut url = cfg.ws_url.clone();
    // 查询参数带 key，避免依赖 on_open 时序
    if !url.contains("key=") {
        let sep = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{sep}key={token}");
    }

    info!(%url, "jian: spawn ws");
    tokio::spawn(async move {
        run_ws_loop(
            tx,
            WsLoopOpts {
                source: "jian",
                url,
                on_open_text: Some("alllist".into()),
                backoff_max_secs: 60,
            },
            |tx, text| ingest_jian_frame(tx, text),
        )
        .await;
    });
}
