//! 中央台气象预警：Jian `/weather` WebSocket。

use starquake_config::{JianSourceConfig, WeatherSourceConfig};
use starquake_core::WeatherAlert;
use serde_json::Value;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bus::{NetEvent, NetTx};
use crate::sources::jian::make_resolve_url;
use crate::sources::ws::{redact_url, run_ws_loop, WsLoopOpts};
use crate::time_parse::parse_time_str;

fn has_jian_credentials(cfg: &JianSourceConfig) -> bool {
    cfg.refresh_token
        .as_ref()
        .map(|s| s.trim().starts_with("rt_"))
        .unwrap_or(false)
        || cfg
            .access_token
            .as_ref()
            .map(|s| s.trim().starts_with("at_"))
            .unwrap_or(false)
}

pub fn spawn(
    tx: NetTx,
    weather: WeatherSourceConfig,
    jian: JianSourceConfig,
) -> Vec<JoinHandle<()>> {
    if !weather.enabled {
        return Vec::new();
    }
    if !has_jian_credentials(&jian) {
        warn!("weather: 无 at_/rt_，跳过 /weather");
        crate::bus::emit_health(&tx, "weather", starquake_core::HealthStatus::Abnormal);
        return Vec::new();
    }

    let base_url = weather.ws_url.clone();
    info!(url = %redact_url(&base_url), "weather: spawn");
    let resolve = make_resolve_url(base_url.clone(), jian);
    let tx_w = tx.clone();
    vec![tokio::spawn(async move {
        run_ws_loop(
            tx_w,
            WsLoopOpts {
                source: "weather",
                url: base_url,
                resolve_url: Some(resolve),
                on_open_texts: Vec::new(),
                periodic_texts: Vec::new(),
                periodic_secs: 0,
                backoff_max_secs: 60,
                stale_secs: 120,
            },
            |tx, text| ingest_weather_frame(tx, text),
        )
        .await;
    })]
}

pub fn ingest_weather_frame(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if typ == "heartbeat" || typ == "pong" || typ == "error" {
        return;
    }
    // 信封 type=weather；Data 为业务体
    let data = if typ == "weather" {
        v.get("Data").or_else(|| v.get("data")).unwrap_or(&v)
    } else if v.get("title").is_some() && v.get("id").is_some() {
        &v
    } else {
        return;
    };
    if let Some(alert) = parse_weather_alert(data) {
        let _ = tx.send(NetEvent::Weather(alert));
    }
}

fn parse_weather_alert(data: &Value) -> Option<WeatherAlert> {
    let id = data.get("id")?.as_str()?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let title = data
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
    let origin_time = data
        .get("originTime")
        .or_else(|| data.get("origin_time"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let origin_ms = if origin_time.is_empty() {
        0
    } else {
        parse_weather_time(&origin_time).unwrap_or(0)
    };
    let lat = opt_f64(data, "latitude");
    let lon = opt_f64(data, "longitude");
    Some(WeatherAlert {
        id,
        title,
        headline: data
            .get("headline")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        origin_time,
        relieve_time: data
            .get("relieveTime")
            .or_else(|| data.get("relieve_time"))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        province: str_field(data, "province"),
        city: str_field(data, "city"),
        district: str_field(data, "district"),
        adcode: data
            .get("adcode")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        latitude: lat,
        longitude: lon,
        icon_type: data
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string(),
        description: str_field(data, "description"),
        origin_ms,
    })
}

fn str_field(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn opt_f64(data: &Value, key: &str) -> Option<f64> {
    let v = data.get(key)?;
    let n = if let Some(f) = v.as_f64() {
        f
    } else if let Some(s) = v.as_str() {
        s.trim().parse().ok()?
    } else {
        return None;
    };
    if n.is_finite() {
        Some(n)
    } else {
        None
    }
}

/// 气象时间多为 `YYYY/MM/DD HH:mm`（UTC+8）。
fn parse_weather_time(s: &str) -> Option<i64> {
    if let Some(ms) = parse_time_str(s) {
        return Some(ms);
    }
    use chrono::{FixedOffset, NaiveDateTime, TimeZone};
    let s = s.trim();
    for fmt in ["%Y/%m/%d %H:%M", "%Y/%m/%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            let cst = FixedOffset::east_opt(8 * 3600)?;
            return Some(cst.from_local_datetime(&ndt).single()?.timestamp_millis());
        }
    }
    None
}
