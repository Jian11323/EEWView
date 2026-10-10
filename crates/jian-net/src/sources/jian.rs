//! Jian Project API（`api.sismotide.top`）：`/all` 预警情报 + `/kmoni` 测站。

use jian_config::JianSourceConfig;
use jian_core::HealthStatus;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::adapter::{ingest_jian_frame, ingest_kmoni_frame, KmoniState};
use crate::auth::exchange_access_token;
use crate::bus::{emit_health, NetTx};
use crate::sources::ws::{run_ws_loop, WsLoopOpts};

fn resolve_access_token(cfg: &JianSourceConfig) -> Option<String> {
    if let Some(at) = cfg.access_token.as_ref().map(|s| s.trim().to_string()) {
        if at.starts_with("at_") {
            return Some(at);
        }
    }
    let rt = cfg.refresh_token.as_ref()?.trim();
    if !rt.starts_with("rt_") {
        return None;
    }
    match std::thread::spawn({
        let rt = rt.to_string();
        move || exchange_access_token(&rt)
    })
    .join()
    .unwrap_or_else(|_| Err("rt_→at_ 线程异常".into()))
    {
        Ok(ok) => {
            info!("jian: 已用 rt_ 换取 at_");
            Some(ok.token)
        }
        Err(e) => {
            warn!(error = %e, "jian: rt_→at_ 失败");
            None
        }
    }
}

pub fn spawn(tx: NetTx, cfg: JianSourceConfig) -> Vec<JoinHandle<()>> {
    let mut tasks = Vec::new();
    let mut any = false;

    if cfg.enabled {
        any = true;
        let token = resolve_access_token(&cfg);
        let Some(token) = token else {
            warn!("jian: 无 at_/rt_（请在设置中用 lk_ 换票），跳过 /all");
            emit_health(&tx, "jian", HealthStatus::Abnormal);
            tasks.extend(spawn_kmoni_if(&tx, &cfg));
            return tasks;
        };

        let mut url = cfg.ws_url.clone();
        if !url.contains("key=") {
            let sep = if url.contains('?') { '&' } else { '?' };
            url = format!("{url}{sep}key={token}");
        }

        info!(%url, "jian: spawn /all");
        let tx_all = tx.clone();
        tasks.push(tokio::spawn(async move {
            run_ws_loop(
                tx_all,
                WsLoopOpts {
                    source: "jian",
                    url,
                    on_open_text: Some("alllist".into()),
                    backoff_max_secs: 60,
                },
                |tx, text| ingest_jian_frame(tx, text),
            )
            .await;
        }));
    }

    if cfg.station_enabled {
        any = true;
        tasks.extend(spawn_kmoni_if(&tx, &cfg));
    }

    if !any {
        emit_health(&tx, "jian", HealthStatus::Abnormal);
    }
    tasks
}

fn spawn_kmoni_if(tx: &NetTx, cfg: &JianSourceConfig) -> Vec<JoinHandle<()>> {
    if !cfg.station_enabled {
        return Vec::new();
    }
    let url = cfg.station_ws_url.clone();
    info!(%url, "jian: spawn kmoni");
    let tx = tx.clone();
    vec![tokio::spawn(async move {
        let mut state = KmoniState::default();
        run_ws_loop(
            tx,
            WsLoopOpts {
                source: "kmoni",
                url,
                on_open_text: None,
                backoff_max_secs: 60,
            },
            move |tx, text| ingest_kmoni_frame(&mut state, tx, text),
        )
        .await;
    })]
}
