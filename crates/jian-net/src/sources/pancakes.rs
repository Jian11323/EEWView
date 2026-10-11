//! PancakesAPI（api.aloys23.link）：全源聚合 WebSocket + HTTP 冷启动。

use jian_config::PancakesSourceConfig;
use jian_core::HealthStatus;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::adapter::ingest_pancakes_event;
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

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

    // HTTP 冷启动：地震列表
    let cold = cfg.clone();
    let tx_http = tx.clone();
    let base_http = base.clone();
    tasks.push(tokio::spawn(async move {
        cold_start(tx_http, base_http, cold).await;
    }));

    let ws_base = cfg.ws_url.trim_end_matches('/').to_string();
    let url = if cfg.use_all {
        format!("{ws_base}/api/v1/alert/ws/all?clientId={client_id}")
    } else {
        // 分源时仍优先 all，由 ingest 按开关过滤；避免占满多路连接
        format!("{ws_base}/api/v1/alert/ws/all?clientId={client_id}")
    };
    info!(%url, "pancakes: spawn ws");
    let filter = cfg.clone();
    let tx_ws = tx.clone();
    tasks.push(tokio::spawn(async move {
        run_ws_loop(
            tx_ws,
            WsLoopOpts {
                source: "pancakes",
                url,
                on_open_text: None,
                backoff_max_secs: 60,
            },
            move |tx, text| ingest_pancakes_event(tx, text, &filter),
        )
        .await;
    }));
    tasks
}

async fn cold_start(tx: NetTx, base: String, cfg: PancakesSourceConfig) {
    let client = match reqwest::Client::builder()
        .user_agent("EEWView/0.0.1 (+https://github.com/Jian11323/EEWView)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("pancakes http client: {e}");
            return;
        }
    };

    let paths: &[(&str, bool)] = &[
        ("/api/v1/alert/quake/gq", cfg.gq),
        ("/api/v1/alert/quake/usgs", cfg.usgs),
        ("/api/v1/alert/quake/jma_eew", cfg.jma_eew),
        ("/api/v1/alert/quake/jma_eqlist", cfg.jma_eqlist),
        ("/api/v1/alert/ocean/nmefc", cfg.nmefc),
        ("/api/v1/alert/ocean/nmefc-wave", cfg.nmefc_wave),
        ("/api/v1/alert/ocean/nmefc-surge", cfg.nmefc_surge),
        ("/api/v1/alert/ocean/jma_tsunami", cfg.jma_tsunami),
        ("/api/v1/alert/weather/cma", cfg.cma),
        ("/api/v1/alert/volcano/jma", cfg.jma_volcano),
    ];

    for (path, on) in paths {
        if !*on {
            continue;
        }
        let url = format!("{base}{path}");
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(body) => {
                    info!(%path, bytes = body.len(), "pancakes: cold start");
                    ingest_http_list(&tx, &body, &cfg);
                }
                Err(e) => warn!(%path, "pancakes body: {e}"),
            },
            Ok(resp) => warn!(%path, status = %resp.status(), "pancakes http"),
            Err(e) => warn!(%path, "pancakes http: {e}"),
        }
    }
}

fn ingest_http_list(tx: &NetTx, body: &str, cfg: &PancakesSourceConfig) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return;
    };
    let Some(arr) = v.as_array() else {
        return;
    };
    for item in arr.iter().take(40) {
        // 摘要项补成 RealtimeEvent 外壳再走统一解析
        let source = item
            .get("source")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if source.is_empty() {
            continue;
        }
        let typ = item
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or(default_type_for_source(&source))
            .to_string();
        let envelope = serde_json::json!({
            "source": source,
            "type": typ,
            "action": "update",
            "timestampMs": 0,
            "payload": item,
        });
        if let Ok(text) = serde_json::to_string(&envelope) {
            ingest_pancakes_event(tx, &text, cfg);
        }
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
