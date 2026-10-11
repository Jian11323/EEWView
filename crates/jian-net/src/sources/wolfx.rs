//! Wolfx（`ws-api.wolfx.jp` / `api.wolfx.jp`）。
//!
//! 默认连接 `all_eew` 聚合通道；按消息 `type` 分发各源预警与速报列表。

use jian_config::WolfxSourceConfig;
use jian_core::HealthStatus;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::adapter::{ingest_wolfx_cwa_eew, ingest_wolfx_eqlist, ingest_wolfx_frame};
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

/// CENC 速报列表 HTTP 轮询间隔（秒）；WS `query_cenceqlist` 之外的冷启动补充。
const EQLIST_POLL_SECS: u64 = 45;

pub fn spawn(tx: NetTx, cfg: WolfxSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.enabled {
        emit_health(&tx, "wolfx", HealthStatus::Abnormal);
        return Vec::new();
    }

    let mut tasks = Vec::new();
    if cfg.eqlist_enabled {
        let eqlist = cfg.http_eqlist.clone();
        let tx_http = tx.clone();
        let ws_also = (cfg.eew_enabled || cfg.eqlist_enabled) && !cfg.ws_urls.is_empty();
        tasks.push(tokio::spawn(async move {
            poll_eqlist(tx_http, eqlist, ws_also).await;
        }));
    }

    // 预警或速报任一开启即连 WS（all_eew 可 query 列表）
    if cfg.eew_enabled || cfg.eqlist_enabled {
        for url in cfg.ws_urls {
            let tx_ws = tx.clone();
            let eew = cfg.eew_enabled;
            let eqlist = cfg.eqlist_enabled;
            let is_cwa = url.contains("cwa_eew");
            let on_open = wolfx_on_open_texts(&url, eew, eqlist);
            info!(%url, "wolfx: spawn ws");
            tasks.push(tokio::spawn(async move {
                run_ws_loop(
                    tx_ws,
                    WsLoopOpts {
                        source: "wolfx",
                        url,
                        on_open_texts: on_open,
                        periodic_texts: vec!["ping".into()],
                        periodic_secs: 30,
                        backoff_max_secs: 60,
                    },
                    move |tx, text| {
                        if is_cwa {
                            if eew {
                                ingest_wolfx_cwa_eew(tx, text);
                            }
                        } else {
                            ingest_wolfx_frame(tx, text, eew, eqlist);
                        }
                    },
                )
                .await;
            }));
        }
    }
    tasks
}

fn wolfx_on_open_texts(url: &str, eew_enabled: bool, eqlist_enabled: bool) -> Vec<String> {
    let mut texts = vec!["ping".into()];
    if url.contains("cwa_eew") {
        return texts;
    }
    // all_eew 手动查询：冷启动拉取各源最新报
    if eew_enabled {
        texts.extend([
            "query_jmaeew".into(),
            "query_sceew".into(),
            "query_fjeew".into(),
            "query_cqeew".into(),
            "query_cenceew".into(),
        ]);
    }
    if eqlist_enabled {
        texts.extend(["query_cenceqlist".into(), "query_jmaeqlist".into()]);
    }
    texts
}

async fn poll_eqlist(tx: NetTx, url: String, ws_also: bool) {
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
            warn!("wolfx http client: {e}");
            return;
        }
    };
    info!(%url, poll_secs = EQLIST_POLL_SECS, "wolfx: spawn eqlist poll");
    loop {
        let ok = match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(body) => {
                    ingest_wolfx_eqlist(&tx, &body);
                    true
                }
                Err(e) => {
                    warn!("wolfx eqlist body: {e}");
                    false
                }
            },
            Ok(resp) => {
                warn!(status = %resp.status(), "wolfx eqlist http");
                false
            }
            Err(e) => {
                warn!("wolfx eqlist: {e}");
                false
            }
        };
        // 仅开 HTTP 且无 WS 时由轮询维持健康态；有 WS 时勿互相覆盖
        if !ws_also {
            emit_health(
                &tx,
                "wolfx",
                if ok {
                    HealthStatus::Normal
                } else {
                    HealthStatus::Abnormal
                },
            );
        }
        sleep(Duration::from_secs(EQLIST_POLL_SECS)).await;
    }
}
