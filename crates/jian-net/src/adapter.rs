//! 原始 JSON → core 模型（各源字段在此归一化）。

use jian_core::{
    cea_pr_agency_id, fill_intensity, intensity_kind_for_agency, jma_from_instrumental,
    jma_shindo_level, AgencyId, EewReport, EqRecord, IntensityKind, StationSample,
};
use serde_json::Value;
use tracing::debug;

use crate::bus::{NetEvent, NetTx};
use crate::time_parse::{format_time_text, parse_origin_ms, parse_time_str};

/// JSON 数字或数字字符串 → f64（Wolfx CENC 的 magnitude/lat/lon/depth 常为字符串）。
fn json_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let s = s.trim().trim_end_matches("km").trim_end_matches("KM").trim();
            if s.is_empty() || s == "—" || s == "-" {
                return None;
            }
            s.parse().ok()
        }
        _ => None,
    }
}

fn json_intensity_raw(data: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = data.get(*k) {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.as_f64().map(|f| format!("{f}")).unwrap_or_default(),
                _ => continue,
            };
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn field_f64(data: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = data.get(*k) {
            if let Some(f) = json_f64(v) {
                return Some(f);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Default)]
pub struct KmoniState {
    meta: Vec<KmoniMeta>,
}

#[derive(Debug, Clone)]
struct KmoniMeta {
    code: String,
    name: String,
    lat: f64,
    lon: f64,
}

/// Jian `/kmoni`：首包带 `stations` 元数据，后续 `int[]` 为計測震度（缺测 -3）。
pub fn ingest_kmoni_frame(state: &mut KmoniState, tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" || typ == "error" {
        return;
    }
    if typ != "kmoni" && v.get("int").is_none() && v.get("stations").is_none() {
        return;
    }

    if let Some(arr) = v.get("stations").and_then(|s| s.as_array()) {
        state.meta.clear();
        state.meta.reserve(arr.len());
        for s in arr {
            let code = s
                .get("code")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if code.is_empty() {
                continue;
            }
            let name = s
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or(code.as_str())
                .to_string();
            let lat = field_f64(s, &["lat", "latitude"]).unwrap_or(0.0);
            let lon = field_f64(s, &["lon", "lng", "longitude"]).unwrap_or(0.0);
            state.meta.push(KmoniMeta {
                code,
                name,
                lat,
                lon,
            });
        }
        debug!(n = state.meta.len(), "kmoni: stations meta");
    }

    let Some(ints) = v.get("int").and_then(|i| i.as_array()) else {
        return;
    };
    if state.meta.is_empty() {
        return;
    }

    let mut out = Vec::new();
    let n = ints.len().min(state.meta.len());
    for i in 0..n {
        let raw = ints[i].as_f64().unwrap_or(-3.0);
        if raw <= -3.0 {
            continue;
        }
        // 列表只收有感（≥1）；0 档仍上地图时可再放宽
        if raw < 0.5 {
            continue;
        }
        let m = &state.meta[i];
        let (level, text) = jma_from_instrumental(raw);
        out.push(StationSample {
            id: m.code.clone(),
            name: m.name.clone(),
            latitude: m.lat,
            longitude: m.lon,
            intensity_text: text.into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: level,
        });
    }
    let _ = tx.send(NetEvent::Stations(out));
}

/// Jian `/all` 或单源信封：`{ type, Data, md5 }` 或 `source：xxx`
pub fn ingest_jian_frame(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" {
        return;
    }
    if typ == "error" {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(no message)");
        tracing::warn!(message = msg, "jian: error 帧（常见原因：同令牌/同 IP 并发已满，白名单不增加并发额度）");
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

fn normalize_jian_agency(agency: &str, data: &Value) -> String {
    let a = agency.trim();
    if a.eq_ignore_ascii_case("cea-pr") {
        let province = data.get("province").and_then(|v| v.as_str());
        return cea_pr_agency_id(province);
    }
    a.to_string()
}

fn parse_jian_eew(agency: &str, data: &Value) -> Option<EewReport> {
    let agency = normalize_jian_agency(agency, data);
    let place = data
        .get("placeName")
        .or_else(|| data.get("Hypocenter"))
        .and_then(|v| v.as_str())?
        .to_string();
    let event_id = data
        .get("id")
        .or_else(|| data.get("EventID"))
        .or_else(|| data.get("eventId"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let serial = data
        .get("serial")
        .or_else(|| data.get("number"))
        .or_else(|| data.get("Serial"))
        .or_else(|| data.get("updates"))
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(1) as u32;
    let magnitude = field_f64(data, &["magnitude", "Magunitude", "Magnitude"])?;
    let depth_km = field_f64(data, &["depth", "Depth"]).unwrap_or(10.0);
    let latitude = field_f64(data, &["latitude", "Latitude"])?;
    let longitude = field_f64(data, &["longitude", "Longitude"])?;
    let origin_ms = data
        .get("originTime")
        .or_else(|| data.get("shockTime"))
        .map(parse_origin_ms)
        .unwrap_or(0);

    let kind = intensity_kind_for_agency(&agency);
    let raw = json_intensity_raw(
        data,
        &["intensity", "MaxIntensity", "maxIntensity", "epiIntensity"],
    );
    let (max_intensity_text, intensity_level) =
        fill_intensity(kind, magnitude, depth_km, raw.as_deref());

    Some(EewReport {
        agency: AgencyId(agency.clone()),
        event_id,
        serial,
        place: crate::place::fix_place(&agency, &place, latitude, longitude),
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms,
        max_intensity_text,
        intensity_kind: kind,
        intensity_level,
        is_final: flag_true(data, &["isFinal", "is_final", "final", "isLast"]),
        is_cancel: flag_true(
            data,
            &["isCancel", "isCanceled", "isCancelled", "canceled", "cancelled", "is_cancel"],
        ),
    })
}

fn flag_true(data: &Value, keys: &[&str]) -> bool {
    for k in keys {
        if let Some(v) = data.get(*k) {
            if json_truthy(v) {
                return true;
            }
        }
    }
    false
}

fn json_truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => {
            let s = s.trim();
            matches!(
                s,
                "1" | "true"
                    | "True"
                    | "TRUE"
                    | "yes"
                    | "cancel"
                    | "canceled"
                    | "cancelled"
                    | "最終"
                    | "最终"
                    | "最終報"
                    | "最终报"
                    | "取消"
                    | "取消報"
            )
        }
        _ => false,
    }
}

fn parse_jian_record(agency: &str, data: &Value) -> Option<EqRecord> {
    let place = data
        .get("placeName")
        .or_else(|| data.get("location"))
        .or_else(|| data.get("Place"))
        .and_then(|v| v.as_str())?
        .to_string();
    let magnitude = field_f64(data, &["magnitude", "Magnitude", "Magunitude"])?;
    let latitude = field_f64(data, &["latitude", "Latitude"])?;
    let longitude = field_f64(data, &["longitude", "Longitude"])?;
    let depth_km = field_f64(data, &["depth", "Depth"]).unwrap_or(10.0);
    let origin_ms = data
        .get("originTime")
        .map(parse_origin_ms)
        .unwrap_or(0);
    let kind = intensity_kind_for_agency(agency);
    let raw = json_intensity_raw(data, &["maxIntensity", "intensity", "MaxIntensity"]);
    let (intensity_text, intensity_level) =
        fill_intensity(kind, magnitude, depth_km, raw.as_deref());

    Some(EqRecord {
        agency: AgencyId(agency.into()),
        place: crate::place::fix_place(agency, &place, latitude, longitude),
        time_text: format_time_text(origin_ms),
        magnitude,
        intensity_text,
        intensity_kind: kind,
        intensity_level,
        latitude,
        longitude,
        depth_km,
        origin_ms,
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
    let magnitude = field_f64(&v, &["Magunitude", "Magnitude", "magnitude"]).unwrap_or(0.0);
    let depth_km = field_f64(&v, &["Depth", "depth"]).unwrap_or(10.0);
    let latitude = field_f64(&v, &["Latitude", "latitude"]).unwrap_or(0.0);
    let longitude = field_f64(&v, &["Longitude", "longitude"]).unwrap_or(0.0);
    let origin_ms = v
        .get("OriginTime")
        .and_then(|x| x.as_str())
        .and_then(parse_time_str)
        .unwrap_or(0);
    let max_t = v
        .get("MaxIntensity")
        .and_then(|x| x.as_str())
        .unwrap_or("—");
    let (max_intensity_text, level) = fill_intensity(
        IntensityKind::JmaShindo,
        magnitude,
        depth_km,
        Some(max_t),
    );
    let is_cancel = flag_true(&v, &["isCancel", "isCanceled", "is_cancel"])
        || v.get("Type")
            .and_then(|x| x.as_str())
            .is_some_and(|s| s.contains("取消"));
    let is_final = !is_cancel
        && (flag_true(&v, &["isFinal", "is_final", "Final"])
            || v.get("Type")
                .and_then(|x| x.as_str())
                .is_some_and(|s| s.contains("最終") || s.contains("最终")));

    let _ = tx.send(NetEvent::Eew(EewReport {
        agency: AgencyId("wolfx-jma-eew".into()),
        event_id,
        serial,
        place: crate::place::fix_place("wolfx-jma-eew", &place, latitude, longitude),
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms,
        max_intensity_text,
        intensity_kind: IntensityKind::JmaShindo,
        intensity_level: level,
        is_final,
        is_cancel,
    }));
}

/// Wolfx `cenc_eqlist.json`：对象键为 No1..NoN 或数组。
/// 注意：官方字段 `magnitude` / `latitude` / `longitude` / `depth` / `intensity` 多为**字符串**。
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
        // No10 须排在 No2 之后：按数字后缀排序
        pairs.sort_by(|(a, _), (b, _)| {
            let na = a.trim_start_matches(['N', 'n', 'O', 'o']).parse::<u32>().unwrap_or(0);
            let nb = b.trim_start_matches(['N', 'n', 'O', 'o']).parse::<u32>().unwrap_or(0);
            na.cmp(&nb)
        });
        pairs.into_iter().map(|(_, val)| val).collect()
    } else {
        return;
    };

    for item in items.into_iter().take(30) {
        let place = item
            .get("placeName")
            .or_else(|| item.get("location"))
            .or_else(|| item.get("place"))
            .or_else(|| item.get("Place"))
            .and_then(|x| x.as_str())
            .unwrap_or("—")
            .to_string();
        let magnitude = field_f64(item, &["magnitude", "Magnitude"]).unwrap_or(0.0);
        let latitude = field_f64(item, &["latitude", "Latitude"]).unwrap_or(0.0);
        let longitude = field_f64(item, &["longitude", "Longitude"]).unwrap_or(0.0);
        let depth_km = field_f64(item, &["depth", "Depth"]).unwrap_or(10.0);
        let origin_ms = item
            .get("time")
            .or_else(|| item.get("Time"))
            .and_then(|x| x.as_str())
            .and_then(parse_time_str)
            .unwrap_or(0);
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

        let raw = json_intensity_raw(item, &["intensity", "maxIntensity"]);
        let (intensity_text, intensity_level) = fill_intensity(
            IntensityKind::CnIntensity,
            magnitude,
            depth_km,
            raw.as_deref(),
        );

        let _ = tx.send(NetEvent::Record(EqRecord {
            agency: AgencyId("wolfx-cenc".into()),
            place: crate::place::fix_place("wolfx-cenc", &place, latitude, longitude),
            time_text,
            magnitude,
            intensity_text,
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level,
            latitude,
            longitude,
            depth_km,
            origin_ms,
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
        let latitude = field_f64(hypo, &["latitude", "Latitude"]).unwrap_or(0.0);
        let longitude = field_f64(hypo, &["longitude", "Longitude"]).unwrap_or(0.0);
        let magnitude = field_f64(hypo, &["magnitude", "Magnitude"]).unwrap_or(0.0);
        let depth_km = field_f64(hypo, &["depth", "Depth"]).unwrap_or(10.0);
        let max_scale = eq.get("maxScale").and_then(|x| x.as_i64());
        let (intensity_text, level) = if let Some(scale) = max_scale {
            let t = jian_core::jma_shindo_text(jma_shindo_level(&scale.to_string()));
            fill_intensity(IntensityKind::JmaShindo, magnitude, depth_km, Some(t))
        } else {
            fill_intensity(IntensityKind::JmaShindo, magnitude, depth_km, None)
        };
        let origin_ms = eq
            .get("time")
            .and_then(|x| x.as_str())
            .and_then(parse_time_str)
            .unwrap_or(0);
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
            place: crate::place::fix_place("p2pquake", place, latitude, longitude),
            time_text,
            magnitude,
            intensity_text,
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: level,
            latitude,
            longitude,
            depth_km,
            origin_ms,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::channel;

    #[test]
    fn wolfx_cenc_parses_string_numbers() {
        let body = r#"{
            "No1": {
                "type": "reviewed",
                "EventID": "CC.test",
                "time": "2026-10-10 12:14:01",
                "location": "巴拿马",
                "placeName": "巴拿马",
                "magnitude": "6.0",
                "depth": "10",
                "latitude": "7.50",
                "longitude": "-80.60",
                "intensity": "8"
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_eqlist(&tx, body);
        let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
            panic!("expected Record");
        };
        assert!((rec.magnitude - 6.0).abs() < 1e-9);
        assert!((rec.latitude - 7.5).abs() < 1e-9);
        assert!((rec.longitude - (-80.6)).abs() < 1e-9);
        assert!((rec.depth_km - 10.0).abs() < 1e-9);
        assert_eq!(rec.intensity_level, 8);
        assert_eq!(rec.place, "巴拿马");
    }
}
