//! 气象叠层：本站全球雷达、全国/全球红外云图。

use jian_core::{CloudSheet, HealthStatus};
use serde_json::Value;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::bus::{emit_health, NetEvent, NetTx};

const RADAR_URL: &str = "https://api.sismotide.top/get/global-radar.php";
const CHINA_CLOUD_URL: &str = "https://api.sismotide.top/get/cloud.php";
const GLOBAL_CLOUD_URL: &str = "https://api.sismotide.top/get/global-cloud.php";
/// 本站 GET 每 IP 至少间隔 1 秒；与台风等共用，客户端须错开。
const GAP: Duration = Duration::from_millis(1200);
/// 用户约定：约 5 分钟一轮。
const POLL_OK: Duration = Duration::from_secs(300);
const POLL_FAIL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Default)]
pub struct WeatherPollOpts {
    pub want_radar: bool,
    pub want_china_cloud: bool,
    pub want_global_cloud: bool,
}

impl WeatherPollOpts {
    pub fn any(self) -> bool {
        self.want_radar || self.want_china_cloud || self.want_global_cloud
    }
}

pub fn spawn(tx: NetTx, opts: WeatherPollOpts) -> Vec<JoinHandle<()>> {
    if !opts.any() {
        return Vec::new();
    }
    info!(
        radar = opts.want_radar,
        china_cloud = opts.want_china_cloud,
        global_cloud = opts.want_global_cloud,
        "weather: spawn overlay poll"
    );
    vec![tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent(concat!(
                "EEWView/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/Jian11323/EEWView)"
            ))
            .timeout(Duration::from_secs(120))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!("weather client: {e}");
                return;
            }
        };

        // 避开启动瞬间与其它 GET（台风等）撞 1s 限流
        sleep(Duration::from_secs(2)).await;

        loop {
            let mut ok_n = 0u32;
            let mut fail_n = 0u32;

            if opts.want_china_cloud {
                match fetch_sheet_retry(&client, CHINA_CLOUD_URL).await {
                    Ok(sheet) => {
                        let _ = tx.send(NetEvent::WeatherChinaCloudSheet(sheet));
                        ok_n += 1;
                    }
                    Err(e) => {
                        warn!("weather china cloud: {e}");
                        fail_n += 1;
                    }
                }
                sleep(GAP).await;
            }

            if opts.want_global_cloud {
                match fetch_sheet_retry(&client, GLOBAL_CLOUD_URL).await {
                    Ok(sheet) => {
                        let _ = tx.send(NetEvent::WeatherGlobalCloudSheet(sheet));
                        ok_n += 1;
                    }
                    Err(e) => {
                        warn!("weather global cloud: {e}");
                        fail_n += 1;
                    }
                }
                sleep(GAP).await;
            }

            if opts.want_radar {
                match fetch_sheet_retry(&client, RADAR_URL).await {
                    Ok(sheet) => {
                        let _ = tx.send(NetEvent::WeatherRadarSheet(sheet));
                        ok_n += 1;
                    }
                    Err(e) => {
                        warn!("weather radar: {e}");
                        fail_n += 1;
                    }
                }
            }

            let status = match (ok_n, fail_n) {
                (0, _) => HealthStatus::Abnormal,
                (_, 0) => HealthStatus::Normal,
                _ => HealthStatus::Fluctuating,
            };
            emit_health(&tx, "weather", status);
            sleep(if ok_n > 0 { POLL_OK } else { POLL_FAIL }).await;
        }
    })]
}

async fn fetch_sheet_retry(client: &reqwest::Client, url: &str) -> Result<CloudSheet, String> {
    let mut last = String::new();
    for attempt in 0..4u32 {
        if attempt > 0 {
            let backoff = Duration::from_secs(2u64.saturating_mul(1u64 << (attempt - 1)));
            sleep(backoff).await;
        }
        match fetch_sheet(client, url).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                let is_429 = e.contains("429");
                last = e;
                if !is_429 {
                    break;
                }
            }
        }
    }
    Err(last)
}

async fn fetch_sheet(client: &reqwest::Client, url: &str) -> Result<CloudSheet, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    parse_image_sheet(&v)
}

fn parse_image_sheet(v: &Value) -> Result<CloudSheet, String> {
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
    if status != "success" {
        return Err("status".into());
    }
    let id = v
        .get("time")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("missing time")?
        .to_string();
    let sw = v
        .pointer("/bounds/sw")
        .and_then(|a| a.as_array())
        .ok_or("missing bounds.sw")?;
    let ne = v
        .pointer("/bounds/ne")
        .and_then(|a| a.as_array())
        .ok_or("missing bounds.ne")?;
    if sw.len() < 2 || ne.len() < 2 {
        return Err("bounds short".into());
    }
    let min_lat = sw[0].as_f64().ok_or("sw lat")?;
    let min_lon = sw[1].as_f64().ok_or("sw lon")?;
    let max_lat = ne[0].as_f64().ok_or("ne lat")?;
    let max_lon = ne[1].as_f64().ok_or("ne lon")?;
    if !(min_lon < max_lon && min_lat < max_lat) {
        return Err("bounds unordered".into());
    }
    let raw = v.get("image").and_then(|s| s.as_str()).ok_or("missing image")?;
    let b64 = raw
        .strip_prefix("data:image/png;base64,")
        .or_else(|| raw.strip_prefix("data:image/jpeg;base64,"))
        .unwrap_or(raw);
    let png = b64_decode(b64)?;
    if png.is_empty() {
        return Err("empty image".into());
    }
    Ok(CloudSheet {
        id,
        png,
        min_lon,
        min_lat,
        max_lon,
        max_lat,
    })
}

fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("bad base64".into()),
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if bytes.len() % 4 != 0 {
        return Err("bad base64 length".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().filter(|c| **c == b'=').count();
        if pad > 2 {
            return Err("bad base64 pad".into());
        }
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = if chunk[2] == b'=' { 0 } else { val(chunk[2])? };
        let d = if chunk[3] == b'=' { 0 } else { val(chunk[3])? };
        out.push((a << 2) | (b >> 4));
        if pad < 2 {
            out.push((b << 4) | (c >> 2));
        }
        if pad < 1 {
            out.push((c << 6) | d);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn image_sheet_uses_radar_fields() {
        let v = json!({
            "status": "success",
            "time": "2026-10-17 21:30:00",
            "bounds": { "sw": [-85.05, -180.0], "ne": [85.05, 180.0] },
            "size": { "width": 8, "height": 8 },
            "image": "data:image/png;base64,aGk="
        });
        let s = parse_image_sheet(&v).expect("sheet");
        assert_eq!(s.png, b"hi");
        assert_eq!(s.id, "2026-10-17 21:30:00");
        assert_eq!(s.min_lat, -85.05);
        assert_eq!(s.max_lon, 180.0);
    }
}
