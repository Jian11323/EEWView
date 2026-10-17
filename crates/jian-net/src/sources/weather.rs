//! 全球气象叠层：RainViewer 雷达 / 云图 path。

use jian_core::HealthStatus;
use serde_json::Value;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::bus::{emit_health, NetEvent, NetTx};

const RAINVIEWER_MAPS: &str = "https://api.rainviewer.com/public/weather-maps.json";

#[derive(Clone, Copy)]
pub struct WeatherPollOpts {
    pub want_radar: bool,
    pub want_cloud: bool,
}

pub fn spawn(tx: NetTx, opts: WeatherPollOpts) -> Vec<JoinHandle<()>> {
    if !opts.want_radar && !opts.want_cloud {
        return Vec::new();
    }
    info!(
        radar = opts.want_radar,
        cloud = opts.want_cloud,
        "weather: spawn overlay poll"
    );
    vec![tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(concat!(
                "EEWView/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/Jian11323/EEWView)"
            ))
            .timeout(Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("weather client: {e}");
                return;
            }
        };

        loop {
            match fetch_rainviewer(&client).await {
                Ok((host, radar, cloud)) => {
                    let _ = tx.send(NetEvent::WeatherTiles {
                        host,
                        radar_path: if opts.want_radar { radar } else { None },
                        cloud_path: if opts.want_cloud { cloud } else { None },
                    });
                    emit_health(&tx, "weather", HealthStatus::Normal);
                }
                Err(e) => {
                    warn!("weather rainviewer: {e}");
                    emit_health(&tx, "weather", HealthStatus::Abnormal);
                }
            }
            sleep(Duration::from_secs(300)).await;
        }
    })]
}

async fn fetch_rainviewer(
    client: &reqwest::Client,
) -> Result<(String, Option<String>, Option<String>), String> {
    let resp = client
        .get(RAINVIEWER_MAPS)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let host = v
        .get("host")
        .and_then(|h| h.as_str())
        .unwrap_or("https://tilecache.rainviewer.com")
        .trim_end_matches('/')
        .to_string();
    let radar = v
        .pointer("/radar/past")
        .and_then(|a| a.as_array())
        .and_then(|a| a.last())
        .and_then(|o| o.get("path"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    let cloud = v
        .pointer("/satellite/infrared")
        .and_then(|a| a.as_array())
        .and_then(|a| a.last())
        .and_then(|o| o.get("path"))
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    Ok((host, radar, cloud))
}
