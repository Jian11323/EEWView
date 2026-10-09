//! Wolfx（`ws-api.wolfx.jp` / `api.wolfx.jp`）。

use jian_config::WolfxSourceConfig;
use jian_core::HealthStatus;
use tracing::{info, warn};

use crate::adapter::{ingest_wolfx_eqlist, ingest_wolfx_jma_eew};
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

pub fn spawn(tx: NetTx, cfg: WolfxSourceConfig) {
    if !cfg.enabled {
        emit_health(&tx, "wolfx", HealthStatus::Abnormal);
        return;
    }

    // HTTP 冷启动速报
    let eqlist = cfg.http_eqlist.clone();
    let tx_http = tx.clone();
    tokio::spawn(async move {
        cold_start_eqlist(tx_http, eqlist).await;
    });

    for url in cfg.ws_urls {
        let tx_ws = tx.clone();
        info!(%url, "wolfx: spawn ws");
        tokio::spawn(async move {
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
        });
    }
}

async fn cold_start_eqlist(tx: NetTx, url: String) {
    let client = match reqwest::Client::builder()
        .user_agent("jian-project/0.1 (+https://github.com/)")
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
