//! 原始 JSON → core 模型（各源字段在此归一化）。

use jian_core::{
    cn_intensity_level, cn_intensity_text, intensity_kind_for_agency, jma_shindo_level,
    jma_shindo_text, AgencyId, EewReport, EqRecord, IntensityKind,
};
use serde_json::Value;
use tracing::debug;

use crate::bus::{NetEvent, NetTx};
use crate::time_parse::{format_time_text, parse_origin_ms, parse_time_str};

/// Jian `/all` 或单源信封：`{ type, Data, md5 }` 或 `source：xxx`
pub fn ingest_jian_frame(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if v.get("type").and_then(|t| t.as_str()) == Some("heartbeat")
        || v.get("type").and_then(|t| t.as_str()) == Some("pong")
        || v.get("type").and_then(|t| t.as_str()) == Some("error")
    {
        return;
    }

    // /all 快照：键形如 "source：cenc"
    if v.get("type").and_then(|t| t.as_str()) == Some("all")
        || v.as_object()
            .map(|o| o.keys().any(|k| k.starts_with("source")))
            .unwrap_or(false)
    {
        if let Some(obj) = v.as_object() {
            for (k, entry) in obj {
                if !k.starts_with("source") {
                    continue;
                }
                let agency = k.split('：').nth(1).or_else(|| k.split(':').nth(1)).unwrap_or(k);
                let data = entry.get("Data").cloned().unwrap_or_else(|| entry.clone());
                dispatch_jian_data(tx, agency, &data);
            }
        }
        return;
    }

    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ.ends_with("list_response") {
        let agency = typ.trim_end_matches("list_response");
        if let Some(arr) = v.get("Data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(rec) = parse_jian_record(agency, item) {
                    let _ = tx.send(NetEvent::Record(rec));
                }
            }
        }
        return;
    }

    let data = v.get("Data").cloned().unwrap_or(v.clone());
    dispatch_jian_data(tx, typ, &data);
}

fn dispatch_jian_data(tx: &NetTx, agency: &str, data: &Value) {
    if data.is_null() || !data.is_object() {
        return;
    }
    let agency = agency.trim();
    if is_eew_agency(agency) {
        if let Some(eew) = parse_jian_eew(agency, data) {
            let _ = tx.send(NetEvent::Eew(eew));
        }
    } else if let Some(rec) = parse_jian_record(agency, data) {
        let _ = tx.send(NetEvent::Record(rec));
    } else {
        debug!(agency, "jian: 未能归一化帧");
    }
}

fn is_eew_agency(agency: &str) -> bool {
    matches!(
        agency,
        "jma-eew" | "cea" | "cea-pr" | "cwa-eew" | "sa" | "kma-eew" | "early-est"
    ) || agency.ends_with("-eew")
}

fn parse_jian_eew(agency: &str, data: &Value) -> Option<EewReport> {
    let place = data
        .get("placeName")
        .or_else(|| data.get("Hypocenter"))
        .and_then(|v| v.as_str())?
        .to_string();
    let event_id = data
        .get("id")
        .or_else(|| data.get("EventID"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let serial = data
        .get("serial")
        .or_else(|| data.get("number"))
        .or_else(|| data.get("Serial"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let magnitude = data
        .get("magnitude")
        .or_else(|| data.get("Magunitude"))
        .and_then(|v| v.as_f64())?;
    let depth_km = data.get("depth").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let latitude = data.get("latitude").and_then(|v| v.as_f64())?;
    let longitude = data.get("longitude").and_then(|v| v.as_f64())?;
    let origin_ms = data
        .get("originTime")
        .map(parse_origin_ms)
        .unwrap_or(0);

    let kind = intensity_kind_for_agency(agency);
    let (max_intensity_text, intensity_level) = match kind {
        IntensityKind::JmaShindo => {
            let t = data
                .get("intensity")
                .or_else(|| data.get("MaxIntensity"))
                .and_then(|v| v.as_str())
                .unwrap_or("—");
            let level = jma_shindo_level(t);
            (if t == "—" { jma_shindo_text(level).into() } else { t.into() }, level)
        }
        IntensityKind::CnIntensity => {
            // CEA 等常无烈度字段 → 占位，不上伪色档
            ("—".into(), 0)
        }
    };

    Some(EewReport {
        agency: AgencyId(agency.into()),
        event_id,
        serial,
        place,
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms,
        max_intensity_text,
        intensity_kind: kind,
        intensity_level,
    })
}

fn parse_jian_record(agency: &str, data: &Value) -> Option<EqRecord> {
    let place = data.get("placeName").and_then(|v| v.as_str())?.to_string();
    let magnitude = data.get("magnitude").and_then(|v| v.as_f64())?;
    let latitude = data.get("latitude").and_then(|v| v.as_f64())?;
    let longitude = data.get("longitude").and_then(|v| v.as_f64())?;
    let depth_km = data.get("depth").and_then(|v| v.as_f64()).unwrap_or(10.0);
    let origin_ms = data
        .get("originTime")
        .map(parse_origin_ms)
        .unwrap_or(0);
    let kind = intensity_kind_for_agency(agency);
    let (intensity_text, intensity_level) = match kind {
        IntensityKind::JmaShindo => {
            let t = data
                .get("intensity")
                .or_else(|| data.get("maxIntensity"))
                .and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else {
                        v.as_f64().map(|f| format!("{f}"))
                    }
                })
                .unwrap_or_else(|| "—".into());
            let level = jma_shindo_level(&t);
            (t, level)
        }
        IntensityKind::CnIntensity => {
            if let Some(f) = data
                .get("maxIntensity")
                .or_else(|| data.get("intensity"))
                .and_then(|v| v.as_f64())
            {
                let level = cn_intensity_level(&format!("{f}"));
                (cn_intensity_text(level).into(), level)
            } else {
                ("—".into(), 0)
            }
        }
    };

    Some(EqRecord {
        agency: AgencyId(agency.into()),
        place,
        time_text: format_time_text(origin_ms),
        magnitude,
        intensity_text,
        intensity_kind: kind,
        intensity_level,
        latitude,
        longitude,
        depth_km,
    })
}

/// Wolfx `jma_eew` 扁平 JSON
pub fn ingest_wolfx_jma_eew(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if v.get("type").and_then(|t| t.as_str()) == Some("heartbeat")
        || v.get("type").and_then(|t| t.as_str()) == Some("pong")
    {
        return;
    }
    // 心跳有时只有简短对象
    if v.get("EventID").is_none() && v.get("Hypocenter").is_none() {
        return;
    }
    let place = v
        .get("Hypocenter")
        .and_then(|x| x.as_str())
        .unwrap_or("—")
        .to_string();
    let event_id = v
        .get("EventID")
        .and_then(|x| x.as_str())
        .unwrap_or("wolfx-unknown")
        .to_string();
    let serial = v.get("Serial").and_then(|x| x.as_u64()).unwrap_or(1) as u32;
    let magnitude = v
        .get("Magunitude")
        .or_else(|| v.get("Magnitude"))
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let depth_raw = v.get("Depth");
    let depth_km = match depth_raw {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(10.0),
        Some(Value::String(s)) => s.replace("km", "").trim().parse().unwrap_or(10.0),
        _ => 10.0,
    };
    let latitude = v.get("Latitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let longitude = v.get("Longitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let origin_ms = v
        .get("OriginTime")
        .and_then(|x| x.as_str())
        .and_then(parse_time_str)
        .unwrap_or(0);
    let max_t = v
        .get("MaxIntensity")
        .and_then(|x| x.as_str())
        .unwrap_or("—");
    let level = jma_shindo_level(max_t);

    let _ = tx.send(NetEvent::Eew(EewReport {
        agency: AgencyId("wolfx-jma-eew".into()),
        event_id,
        serial,
        place,
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms,
        max_intensity_text: if max_t == "—" {
            jma_shindo_text(level).into()
        } else {
            max_t.into()
        },
        intensity_kind: IntensityKind::JmaShindo,
        intensity_level: level,
    }));
}

/// Wolfx `cenc_eqlist.json`：对象键为 No1..NoN 或数组
pub fn ingest_wolfx_eqlist(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let items: Vec<&Value> = if let Some(arr) = v.as_array() {
        arr.iter().collect()
    } else if let Some(obj) = v.as_object() {
        let mut pairs: Vec<_> = obj
            .iter()
            .filter(|(k, _)| k.starts_with("No") || k.starts_with("no"))
            .collect();
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        pairs.into_iter().map(|(_, val)| val).collect()
    } else {
        return;
    };

    for item in items.into_iter().take(30) {
        let place = item
            .get("location")
            .or_else(|| item.get("place"))
            .or_else(|| item.get("Place"))
            .and_then(|x| x.as_str())
            .unwrap_or("—")
            .to_string();
        let magnitude = item
            .get("magnitude")
            .or_else(|| item.get("Magnitude"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let latitude = item
            .get("latitude")
            .or_else(|| item.get("Latitude"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let longitude = item
            .get("longitude")
            .or_else(|| item.get("Longitude"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let depth_km = item
            .get("depth")
            .or_else(|| item.get("Depth"))
            .and_then(|x| match x {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.replace("km", "").trim().parse().ok(),
                _ => None,
            })
            .unwrap_or(10.0);
        let time_text = item
            .get("time")
            .or_else(|| item.get("Time"))
            .and_then(|x| x.as_str())
            .map(|s| {
                parse_time_str(s)
                    .map(format_time_text)
                    .unwrap_or_else(|| s.to_string())
            })
            .unwrap_or_else(|| "—".into());

        let _ = tx.send(NetEvent::Record(EqRecord {
            agency: AgencyId("wolfx-cenc".into()),
            place,
            time_text,
            magnitude,
            intensity_text: "—".into(),
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: 0,
            latitude,
            longitude,
            depth_km,
        }));
    }
}

/// P2PQuake JSON API v2：`code` 551 地震情报
pub fn ingest_p2p_items(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let items: Vec<&Value> = if let Some(arr) = v.as_array() {
        arr.iter().collect()
    } else {
        vec![&v]
    };

    for item in items {
        let code = item.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
        if code != 551 {
            continue;
        }
        let Some(eq) = item.get("earthquake") else {
            continue;
        };
        let Some(hypo) = eq.get("hypocenter") else {
            continue;
        };
        let place = hypo.get("name").and_then(|x| x.as_str()).unwrap_or("—");
        let latitude = hypo.get("latitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let longitude = hypo.get("longitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let magnitude = hypo.get("magnitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let depth_km = hypo.get("depth").and_then(|x| x.as_f64()).unwrap_or(10.0);
        let max_scale = eq.get("maxScale").and_then(|x| x.as_i64()).unwrap_or(0);
        let level = jma_shindo_level(&max_scale.to_string());
        let time_text = eq
            .get("time")
            .and_then(|x| x.as_str())
            .map(|s| {
                parse_time_str(s)
                    .map(format_time_text)
                    .unwrap_or_else(|| s.to_string())
            })
            .unwrap_or_else(|| "—".into());

        let _ = tx.send(NetEvent::Record(EqRecord {
            agency: AgencyId("p2pquake".into()),
            place: place.into(),
            time_text,
            magnitude,
            intensity_text: jma_shindo_text(level).into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: level,
            latitude,
            longitude,
            depth_km,
        }));
    }
}
