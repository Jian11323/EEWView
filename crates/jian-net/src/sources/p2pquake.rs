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
        emit_health(&tx, "p2pquake", HealthStatus::Abnormal);
        return Vec::new();
    }

    let poll = cfg.poll_secs.max(10);
    let base = cfg.http_history.clone();
    info!(%base, poll_secs = poll, "p2pquake: spawn http poll");

    vec![tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent("EEWView/0.0.1 (+https://github.com/Jian11323/EEWView)")
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
            let url = format!("{base}?codes=551&limit=20");
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.text().await {
                    Ok(body) => {
                        ingest_p2p_items(&tx, &body);
                        emit_health(&tx, "p2pquake", HealthStatus::Normal);
                    }
                    Err(e) => {
                        warn!("p2p body: {e}");
                        emit_health(&tx, "p2pquake", HealthStatus::Abnormal);
                    }
                },
                Ok(resp) => {
                    warn!(status = %resp.status(), "p2p http");
                    emit_health(&tx, "p2pquake", HealthStatus::Abnormal);
                }
                Err(e) => {
                    warn!("p2p fetch: {e}");
                    emit_health(&tx, "p2pquake", HealthStatus::Abnormal);
                }
            }
            sleep(Duration::from_secs(poll)).await;
        }
    })]
}
