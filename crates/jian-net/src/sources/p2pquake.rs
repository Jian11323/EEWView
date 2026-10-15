//! P2PQuake JSON API v2（HTTP 轮询）。

use jian_config::P2pSourceConfig;
use jian_core::HealthStatus;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::adapter::ingest_p2p_items;
use crate::bus::{emit_health, NetTx};

pub fn spawn(tx: NetTx, cfg: P2pSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        // 关闭源不标「异常」，由 UI 显示「关闭」
        return Vec::new();
    }

    let poll = cfg.poll_secs.max(10);
    let base = cfg.http_history.clone();
    info!(%base, poll_secs = poll, "p2pquake: spawn http poll");

    vec![tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(concat!("EEWView/", env!("CARGO_PKG_VERSION"), " (+https://github.com/Jian11323/EEWView)"))
            .timeout(Duration::from_secs(25))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("p2p client: {e}");
                emit_health(&tx, "p2pquake", HealthStatus::Abnormal);
                return;
            }
        };

        loop {
            emit_health(&tx, "p2pquake", HealthStatus::Fluctuating);
            // 仅 551 地震情报；海啸改走 Jian Project API（jma-tsunami）
            let url = format!("{base}?codes=551&limit=20");
            let ok_any = match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(body) => {
                        ingest_p2p_items(&tx, &body);
                        true
                    }
                    Err(e) => {
                        warn!("p2p body: {e}");
                        false
                    }
                },
                Ok(resp) => {
                    warn!(status = %resp.status(), "p2p http");
                    false
                }
                Err(e) => {
                    warn!("p2p fetch: {e}");
                    false
                }
            };
            emit_health(
                &tx,
                "p2pquake",
                if ok_any {
                    HealthStatus::Normal
                } else {
                    HealthStatus::Abnormal
                },
            );
            sleep(Duration::from_secs(poll)).await;
        }
    })]
}
