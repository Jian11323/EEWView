//! Wolfx（`ws-api.wolfx.jp` / `api.wolfx.jp`）。

use jian_config::WolfxSourceConfig;
use jian_core::HealthStatus;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::adapter::{ingest_wolfx_eqlist, ingest_wolfx_jma_eew};
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

pub fn spawn(tx: NetTx, cfg: WolfxSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        emit_health(&tx, "wolfx", HealthStatus::Abnormal);
        return Vec::new();
    }

    let mut tasks = Vec::new();
    if cfg.eqlist_enabled {
        let eqlist = cfg.http_eqlist.clone();
        let tx_http = tx.clone();
        tasks.push(tokio::spawn(async move {
            cold_start_eqlist(tx_http, eqlist).await;
        }));
    }

    if cfg.eew_enabled {
        for url in cfg.ws_urls {
            let tx_ws = tx.clone();
            info!(%url, "wolfx: spawn ws");
            tasks.push(tokio::spawn(async move {
                run_ws_loop(
                    tx_ws,
                    WsLoopOpts {
                        source: "wolfx",
                        url,
                        on_open_text: Some("ping".into()),
                        backoff_max_secs: 60,
                    },
                    |tx, text| ingest_wolfx_jma_eew(tx, text),
                )
                .await;
            }));
        }
    }
    tasks
}

async fn cold_start_eqlist(tx: NetTx, url: String) {
    let client = match reqwest::Client::builder()
        .user_agent("EEWView/0.0.1 (+https://github.com/Jian11323/EEWView)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("wolfx http client: {e}");
            return;
        }
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => {
                info!(bytes = body.len(), "wolfx: eqlist cold start");
                ingest_wolfx_eqlist(&tx, &body);
            }
            Err(e) => warn!("wolfx eqlist body: {e}"),
        },
        Ok(resp) => warn!(status = %resp.status(), "wolfx eqlist http"),
        Err(e) => warn!("wolfx eqlist: {e}"),
    }
}
