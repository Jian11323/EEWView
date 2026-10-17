//! 全球气象叠层：RainViewer 雷达/云图 path + 本站风场格点。

use jian_core::{HealthStatus, WindGrid};
use serde_json::Value;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::bus::{emit_health, NetEvent, NetTx};

const RAINVIEWER_MAPS: &str = "https://api.rainviewer.com/public/weather-maps.json";
const WIND_URL: &str = "https://api.sismotide.top/get/wind.php";

#[derive(Clone, Copy)]
pub struct WeatherPollOpts {
    pub want_radar: bool,
    pub want_cloud: bool,
    pub want_wind: bool,
}

pub fn spawn(tx: NetTx, opts: WeatherPollOpts) -> Vec<JoinHandle<()>> {
    if !opts.want_radar && !opts.want_cloud && !opts.want_wind {
        return Vec::new();
    }
    info!(
        radar = opts.want_radar,
        cloud = opts.want_cloud,
        wind = opts.want_wind,
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

        let mut tick: u64 = 0;
        loop {
            if opts.want_radar || opts.want_cloud {
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
            }
            // 风场体积大，约每 6 次雷达周期（~30–60 min）拉一次
            if opts.want_wind && tick % 6 == 0 {
                match fetch_wind(&client).await {
                    Ok(grid) => {
                        let _ = tx.send(NetEvent::WeatherWind(grid));
                        emit_health(&tx, "weather", HealthStatus::Normal);
                    }
                    Err(e) => {
                        warn!("weather wind: {e}");
                        emit_health(&tx, "weather", HealthStatus::Fluctuating);
                    }
                }
            }
            tick = tick.wrapping_add(1);
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

async fn fetch_wind(client: &reqwest::Client) -> Result<WindGrid, String> {
    let resp = client
        .get(WIND_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    parse_wind_json(&v)
}

fn parse_wind_json(v: &Value) -> Result<WindGrid, String> {
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
    if status != "success" && !v.get("data").is_some() {
        return Err("wind status not success".into());
    }
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "wind missing data".to_string())?;
    if data.len() < 2 {
        return Err("wind data needs u,v".into());
    }
    let (nx, ny, lo1, la1, dx, dy, u) = parse_component(&data[0])?;
    let (_, _, _, _, _, _, vv) = parse_component(&data[1])?;
    if u.len() != vv.len() {
        return Err("u/v length mismatch".into());
    }
    Ok(WindGrid {
        time_text: v
            .get("time")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        nx,
        ny,
        lo1,
        la1,
        dx,
        dy,
        u,
        v: vv,
    })
}

fn parse_component(
    layer: &Value,
) -> Result<(usize, usize, f64, f64, f64, f64, Vec<f32>), String> {
    let h = layer
        .get("header")
        .ok_or_else(|| "missing header".to_string())?;
    let nx = h.get("nx").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let ny = h.get("ny").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    if nx == 0 || ny == 0 {
        return Err("nx/ny zero".into());
    }
    let lo1 = h.get("lo1").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let la1 = h.get("la1").and_then(|x| x.as_f64()).unwrap_or(90.0);
    let dx = h.get("dx").and_then(|x| x.as_f64()).unwrap_or(1.0);
    let dy = h.get("dy").and_then(|x| x.as_f64()).unwrap_or(1.0).abs();
    let arr = layer
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| "missing component data".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for x in arr {
        out.push(match x {
            Value::Null => f32::NAN,
            Value::Number(n) => n.as_f64().unwrap_or(f64::NAN) as f32,
            _ => f32::NAN,
        });
    }
    if out.len() != nx * ny {
        return Err(format!("grid size {} != {}x{}", out.len(), nx, ny));
    }
    Ok((nx, ny, lo1, la1, dx, dy, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_small_wind_grid() {
        let raw = r#"{
            "status":"success",
            "time":"2026-10-17 20:00:00",
            "data":[
              {"header":{"nx":2,"ny":1,"lo1":0,"la1":90,"dx":1,"dy":1},"data":[1.5,null]},
              {"header":{"nx":2,"ny":1,"lo1":0,"la1":90,"dx":1,"dy":1},"data":[-0.5,2.0]}
            ]
        }"#;
        let v: Value = serde_json::from_str(raw).unwrap();
        let g = parse_wind_json(&v).unwrap();
        assert_eq!(g.nx, 2);
        assert_eq!(g.time_text, "2026-10-17 20:00:00");
        assert_eq!(g.sample(0.2, 89.5), Some((1.5, -0.5)));
        assert!(g.sample(1.2, 89.5).is_none());
    }

    #[test]
    fn reject_short_wind() {
        let v: Value = serde_json::json!({"status":"success","data":[]});
        assert!(parse_wind_json(&v).is_err());
    }
}
