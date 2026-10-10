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
            // 551 地震情报；552 海啸情报（API 不接受逗号多 code，分开拉）
            let mut ok_any = false;
            for (codes, limit) in [("551", 20u32), ("552", 10u32)] {
                let url = format!("{base}?codes={codes}&limit={limit}");
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => match resp.text().await {
                        Ok(body) => {
                            ingest_p2p_items(&tx, &body);
                            ok_any = true;
                        }
                        Err(e) => warn!(%codes, "p2p body: {e}"),
                    },
                    Ok(resp) => warn!(%codes, status = %resp.status(), "p2p http"),
                    Err(e) => warn!(%codes, "p2p fetch: {e}"),
                }
            }
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
