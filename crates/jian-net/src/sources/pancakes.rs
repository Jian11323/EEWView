//! PancakesAPI（api.aloys23.link）：默认只连 GlobalQuake（gq）专用通道。
//! 维护者说明：其余子源已在上游关闭；可选开关仅作兼容保留。

use jian_config::PancakesSourceConfig;
use jian_core::HealthStatus;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::adapter::ingest_pancakes_event;
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

/// 速报类 HTTP 列表轮询间隔（秒）。
const RECORD_POLL_SECS: u64 = 45;

pub fn spawn(tx: NetTx, cfg: PancakesSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        emit_health(&tx, "pancakes", HealthStatus::Abnormal);
        return Vec::new();
    }

    let mut tasks = Vec::new();
    let base = cfg.base_url.trim_end_matches('/').to_string();
    let client_id = if cfg.client_id.trim().is_empty() {
        "eewview".into()
    } else {
        cfg.client_id.trim().to_string()
    };

    // HTTP：冷启动 / 可选速报轮询（默认仅 gq）
    if needs_http(&cfg) {
        let cold = cfg.clone();
        let tx_http = tx.clone();
        let base_http = base.clone();
        tasks.push(tokio::spawn(async move {
            poll_and_cold(tx_http, base_http, cold).await;
        }));
    }

    let ws_base = cfg.ws_url.trim_end_matches('/').to_string();
    // 维护者：单独连 GQ；`use_all` 仅兼容旧聚合通道
    if cfg.gq || cfg.use_all {
        let url = if cfg.use_all {
            format!("{ws_base}/api/v1/alert/ws/all?clientId={client_id}")
        } else {
            format!("{ws_base}/api/v1/alert/ws/gq?clientId={client_id}")
        };
        info!(%url, use_all = cfg.use_all, "pancakes: spawn ws");
        let filter = cfg.clone();
        let tx_ws = tx.clone();
        tasks.push(tokio::spawn(async move {
            run_ws_loop(
                tx_ws,
                WsLoopOpts {
                    source: "pancakes",
                    url,
                    on_open_texts: Vec::new(),
                    periodic_texts: Vec::new(),
                    periodic_secs: 0,
                    backoff_max_secs: 60,
                },
                move |tx, text| ingest_pancakes_event(tx, text, &filter),
            )
            .await;
        }));
    } else {
        warn!("pancakes: enabled but gq off and use_all off → no websocket");
    }

    tasks
}

fn needs_http(cfg: &PancakesSourceConfig) -> bool {
    cfg.gq
        || cfg.usgs
        || cfg.jma_eew
        || cfg.jma_eqlist
        || cfg.nmefc
        || cfg.nmefc_wave
        || cfg.nmefc_surge
        || cfg.jma_tsunami
        || cfg.cma
        || cfg.jma_volcano
}

async fn poll_and_cold(tx: NetTx, base: String, cfg: PancakesSourceConfig) {
    let client = match reqwest::Client::builder()
        .user_agent(concat!(
            "EEWView/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/Jian11323/EEWView)"
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

    // 非速报：冷启动一次（默认仅 gq）
    let once: &[(&str, bool)] = &[
        ("/api/v1/alert/quake/gq", cfg.gq),
        ("/api/v1/alert/quake/jma_eew", cfg.jma_eew),
        ("/api/v1/alert/ocean/nmefc", cfg.nmefc),
        ("/api/v1/alert/ocean/nmefc-wave", cfg.nmefc_wave),
        ("/api/v1/alert/ocean/nmefc-surge", cfg.nmefc_surge),
        ("/api/v1/alert/ocean/jma_tsunami", cfg.jma_tsunami),
        ("/api/v1/alert/weather/cma", cfg.cma),
        ("/api/v1/alert/volcano/jma", cfg.jma_volcano),
    ];
    for (path, on) in once {
        if *on {
            fetch_path(&client, &tx, &base, path, &cfg).await;
        }
    }

    // 速报：仅当对应开关开启时周期轮询（默认全关）
    let record_paths: &[(&str, bool)] = &[
        ("/api/v1/alert/quake/usgs", cfg.usgs),
        ("/api/v1/alert/quake/jma_eqlist", cfg.jma_eqlist),
    ];
    if !record_paths.iter().any(|(_, on)| *on) {
        return;
    }
    info!(poll_secs = RECORD_POLL_SECS, "pancakes: spawn record list poll");
    loop {
        for (path, on) in record_paths {
            if *on {
                fetch_path(&client, &tx, &base, path, &cfg).await;
            }
        }
        sleep(Duration::from_secs(RECORD_POLL_SECS)).await;
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
        "usgs" => "usgs",
        "jma_eew" => "jma_eew",
        "jma_eqlist" => "jma_eqlist",
        "nmefc" => "nmefc",
        "nmefc-wave" => "nmefc-wave",
        "nmefc-surge" => "nmefc-surge",
        "jma_tsunami" => "jma_tsunami",
        "cma" => "cma",
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
            .unwrap_or(default_type_for_source(&source))
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

fn default_type_for_source(source: &str) -> &'static str {
    match source {
        "nmefc" | "jma_tsunami" => "tsunami",
        "nmefc-wave" | "nmefc-surge" => "alert",
        "cma" => "weather",
        "jma_volcano" => "volcano",
        _ => "earthquake",
    }
}
