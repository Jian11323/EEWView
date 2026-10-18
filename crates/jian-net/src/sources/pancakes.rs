//! PancakesAPI（api.aloys23.link）：仅 GQ + JMA-Atom（jma_volcano）；通道 `/ws/all`。

use jian_config::PancakesSourceConfig;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::adapter::ingest_pancakes_event;
use crate::bus::NetTx;
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

pub fn spawn(tx: NetTx, cfg: PancakesSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        // 关闭源不标「异常」，由 UI 显示「关闭」
        return Vec::new();
    }

    let mut tasks = Vec::new();
    let base = cfg.base_url.trim_end_matches('/').to_string();
    let client_id = if cfg.client_id.trim().is_empty() {
        "starquake".into()
    } else {
        cfg.client_id.trim().to_string()
    };

    if cfg.gq || cfg.jma_volcano {
        let cold = cfg.clone();
        let tx_http = tx.clone();
        let base_http = base.clone();
        tasks.push(tokio::spawn(async move {
            cold_start(tx_http, base_http, cold).await;
        }));
    }

    if cfg.gq || cfg.jma_volcano {
        let ws_base = cfg.ws_url.trim_end_matches('/').to_string();
        let url = format!("{ws_base}/api/v1/alert/ws/all?clientId={client_id}");
        info!(%url, gq = cfg.gq, jma_atom = cfg.jma_volcano, "pancakes: spawn ws");
        let filter = cfg.clone();
        let tx_ws = tx.clone();
        tasks.push(tokio::spawn(async move {
            run_ws_loop(
                tx_ws,
                WsLoopOpts {
                    source: "pancakes",
                    url,
                    resolve_url: None,
                    on_open_texts: Vec::new(),
                    periodic_texts: Vec::new(),
                    periodic_secs: 0,
                    backoff_max_secs: 60,
                    stale_secs: 0,
                },
                move |tx, text| ingest_pancakes_event(tx, text, &filter),
            )
            .await;
        }));
    } else {
        warn!("pancakes: enabled but no sub-source on → no websocket");
    }

    tasks
}

async fn cold_start(tx: NetTx, base: String, cfg: PancakesSourceConfig) {
    let client = match reqwest::Client::builder()
        .user_agent(concat!(
            "StarQuake/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/Jian11323/StarQuake)"
        ))
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("pancakes http client: {e}");
            return;
        }
    };

    let once: &[(&str, bool)] = &[
        ("/api/v1/alert/quake/gq", cfg.gq),
        ("/api/v1/alert/volcano/jma", cfg.jma_volcano),
    ];
    for (path, on) in once {
        if *on {
            fetch_path(&client, &tx, &base, path, &cfg).await;
        }
    }
}

async fn fetch_path(
    client: &reqwest::Client,
    tx: &NetTx,
    base: &str,
    path: &str,
    cfg: &PancakesSourceConfig,
) {
    let url = format!("{base}{path}");
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => {
                let fallback = source_from_path(path);
                ingest_http_list(tx, &body, cfg, fallback);
            }
            Err(e) => warn!(%path, "pancakes body: {e}"),
        },
        Ok(resp) => warn!(%path, status = %resp.status(), "pancakes http"),
        Err(e) => warn!(%path, "pancakes http: {e}"),
    }
}

fn source_from_path(path: &str) -> &'static str {
    match path.rsplit('/').next().unwrap_or("") {
        "gq" => "gq",
        "jma" => "jma_volcano",
        other if other.contains("volcano") => "jma_volcano",
        _ => "",
    }
}

fn ingest_http_list(
    tx: &NetTx,
    body: &str,
    cfg: &PancakesSourceConfig,
    fallback_source: &str,
) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let Some(arr) = v.as_array() else {
        return;
    };
    let mut n = 0u32;
    for item in arr.iter().take(40) {
        let source = item
            .get("source")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback_source)
            .to_string();
        if source.is_empty() {
            continue;
        }
        let typ = item
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or(if source == "jma_volcano" {
                "volcano"
            } else {
                "earthquake"
            })
            .to_string();
        let payload = item.get("payload").cloned().unwrap_or_else(|| item.clone());
        let envelope = serde_json::json!({
            "source": source,
            "type": typ,
            "action": "update",
            "timestampMs": 0,
            "payload": payload,
        });
        if let Ok(text) = serde_json::to_string(&envelope) {
            ingest_pancakes_event(tx, &text, cfg);
            n += 1;
        }
    }
    if n > 0 {
        info!(source = fallback_source, n, "pancakes: http list ingested");
    }
}
