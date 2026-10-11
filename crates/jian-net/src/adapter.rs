//! 原始 JSON → core 模型（各源字段在此归一化）。

use jian_core::{
    cea_pr_agency_id, cn_intensity_text, fill_intensity, instrumental_display_text,
    intensity_kind_for_agency, jma_from_instrumental, jma_shindo_level, AgencyId, EewReport,
    EqRecord, IntensityKind, StationSample, TsunamiArea, TsunamiInfo,
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
        let raw = ints[i].as_f64().unwrap_or(-99.0);
        // < -3 缺测；-3 起全部上图（要石色带最低档深蓝）
        if raw < -3.0 {
            continue;
        }
        let m = &state.meta[i];
        if m.lat.abs() < 1e-6 && m.lon.abs() < 1e-6 {
            continue;
        }
        let (level, _) = jma_from_instrumental(raw);
        out.push(StationSample {
            id: m.code.clone(),
            name: m.name.clone(),
            latitude: m.lat,
            longitude: m.lon,
            intensity_text: instrumental_display_text(raw),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: level,
            instrumental: raw,
        });
    }
    let _ = tx.send(NetEvent::Stations {
        network: "kmoni",
        list: out,
    });
}

/// Jian `/s-net`：协议与 kmoni 相同（`stations` + `int[]`）。
pub fn ingest_snet_frame(state: &mut KmoniState, tx: &NetTx, text: &str) {
    ingest_int_station_frame(state, tx, text, "s-net", "snet", true);
}

/// Jian `/kma-station`：`stations` + `mmi[]`（韩国烈度，映射为 CN 档展示）。
pub fn ingest_kma_station_frame(state: &mut KmoniState, tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" || typ == "error" {
        return;
    }
    if let Some(arr) = v.get("stations").and_then(|s| s.as_array()) {
        load_station_meta(state, arr);
    }
    let Some(mmis) = v.get("mmi").and_then(|i| i.as_array()) else {
        return;
    };
    if state.meta.is_empty() {
        return;
    }
    let mut out = Vec::new();
    let n = mmis.len().min(state.meta.len());
    for i in 0..n {
        let raw = mmis[i].as_f64().unwrap_or(-3.0);
        if raw < 0.0 {
            continue;
        }
        let m = &state.meta[i];
        if m.lat.abs() < 1e-6 && m.lon.abs() < 1e-6 {
            continue;
        }
        let (level, text) = if raw < 1.0 {
            (0_u8, "—".into())
        } else {
            let level = (raw.round() as u8).clamp(1, 12);
            (level, cn_intensity_text(level).into())
        };
        out.push(StationSample {
            id: m.code.clone(),
            name: m.name.clone(),
            latitude: m.lat,
            longitude: m.lon,
            intensity_text: text,
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: level,
            instrumental: raw,
        });
    }
    let _ = tx.send(NetEvent::Stations {
        network: "kma",
        list: out,
    });
}

fn load_station_meta(state: &mut KmoniState, arr: &[Value]) {
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
}

fn ingest_int_station_frame(
    state: &mut KmoniState,
    tx: &NetTx,
    text: &str,
    expect_type: &str,
    network: &'static str,
    jma_scale: bool,
) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" || typ == "error" {
        return;
    }
    if typ != expect_type && v.get("int").is_none() && v.get("stations").is_none() {
        return;
    }
    if let Some(arr) = v.get("stations").and_then(|s| s.as_array()) {
        load_station_meta(state, arr);
        debug!(n = state.meta.len(), %network, "stations meta");
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
        let raw = ints[i].as_f64().unwrap_or(-99.0);
        if raw < -3.0 || raw >= 90.0 {
            continue;
        }
        let m = &state.meta[i];
        if m.lat.abs() < 1e-6 && m.lon.abs() < 1e-6 {
            continue;
        }
        let (kind, level, text) = if jma_scale {
            let (level, _) = jma_from_instrumental(raw);
            (
                IntensityKind::JmaShindo,
                level,
                instrumental_display_text(raw),
            )
        } else if raw < 1.0 {
            (IntensityKind::CnIntensity, 0_u8, "—".into())
        } else {
            let level = (raw.round() as u8).clamp(1, 12);
            (
                IntensityKind::CnIntensity,
                level,
                cn_intensity_text(level).to_string(),
            )
        };
        out.push(StationSample {
            id: m.code.clone(),
            name: m.name.clone(),
            latitude: m.lat,
            longitude: m.lon,
            intensity_text: text,
            intensity_kind: kind,
            intensity_level: level,
            instrumental: raw,
        });
    }
    let _ = tx.send(NetEvent::Stations { network, list: out });
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

    // /all 快照：键形如 "source：cenc"（仅 type=all；勿用 source 前缀误伤其它帧）
    if v.get("type").and_then(|t| t.as_str()) == Some("all") {
        if let Some(obj) = v.as_object() {
            for (k, entry) in obj {
                if !k.starts_with("source") {
                    continue;
                }
                let agency = agency_from_source_key(k);
                let data = entry.get("Data").cloned().unwrap_or_else(|| entry.clone());
                dispatch_jian_data(tx, agency, &data);
            }
        }
        return;
    }

    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ.ends_with("list_response") {
        // strip_suffix：trim_end_matches 按字符集剥离，会把 early-est 裁成 early-
        let agency = agency_from_list_response(typ);
        if let Some(arr) = v.get("Data").and_then(|d| d.as_array()) {
            for item in arr {
                dispatch_jian_data(tx, agency, item);
            }
        }
        return;
    }

    let data = v.get("Data").cloned().unwrap_or(v.clone());
    dispatch_jian_data(tx, typ, &data);
}

fn agency_from_source_key(k: &str) -> &str {
    // source：jma-eew / source:jma-eew / source：cea-pr:四川
    if let Some(rest) = k.strip_prefix("source：").or_else(|| k.strip_prefix("source:")) {
        return rest;
    }
    k.split('：').nth(1).or_else(|| k.split(':').nth(1)).unwrap_or(k)
}

fn dispatch_jian_data(tx: &NetTx, agency: &str, data: &Value) {
    if data.is_null() || !data.is_object() {
        return;
    }
    let agency = agency.trim();
    if is_tsunami_agency(agency) {
        if let Some(ts) = parse_jian_tsunami(agency, data) {
            let _ = tx.send(NetEvent::Tsunami(ts));
        }
        return;
    }
    if is_eew_agency(agency) {
        if let Some(eew) = parse_jian_eew(agency, data) {
            let _ = tx.send(NetEvent::Eew(eew));
        } else {
            tracing::warn!(agency, "jian: EEW 帧未能归一化（缺 placeName/经纬/震级？）");
        }
    } else if let Some(rec) = parse_jian_record(agency, data) {
        let _ = tx.send(NetEvent::Record(rec));
    } else {
        debug!(agency, "jian: 未能归一化帧");
    }
}

fn agency_from_list_response(typ: &str) -> &str {
    typ.strip_suffix("list_response")
        .map(|s| s.strip_suffix('_').unwrap_or(s))
        .unwrap_or(typ)
}

fn is_tsunami_agency(agency: &str) -> bool {
    // 仅 JMA 海啸进顶条+沿岸描边；NMEFC/CENC 等只是报文，不进海啸系统
    let a = agency.trim().to_ascii_lowercase();
    a == "jma-tsunami" || a == "jma_tsunami" || a.ends_with("jma-tsunami")
}

fn parse_jian_tsunami(agency: &str, data: &Value) -> Option<TsunamiInfo> {
    let level = data.get("level").and_then(|v| v.as_str()).unwrap_or("");
    let title_raw = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cancelled = level.contains("解除")
        || title_raw.contains("解除")
        || title_raw.contains("取消")
        || level.eq_ignore_ascii_case("Cancel");
    let grade = if cancelled {
        "Cancel".into()
    } else {
        jian_tsunami_grade(agency, level, &title_raw)
    };
    let zh_title = if cancelled {
        "海啸情报解除".into()
    } else {
        match grade.as_str() {
            "MajorWarning" => "大海啸警报".into(),
            "Warning" => "海啸警报".into(),
            "Watch" => "海啸注意报".into(),
            _ => "海啸预报".into(),
        }
    };
    let area = data
        .get("area")
        .or_else(|| data.get("description"))
        .or_else(|| data.get("place"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
        .to_string();
    let areas_list = parse_jian_tsunami_areas(agency, &area, &grade);
    let areas_summary = if areas_list.is_empty() {
        if area.is_empty() {
            "—".into()
        } else {
            area.chars().take(120).collect()
        }
    } else {
        areas_list
            .iter()
            .take(8)
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("\u{3001}")
    };
    let issued_ms = data.get("originTime").map(parse_origin_ms).unwrap_or(0);
    Some(TsunamiInfo {
        title: zh_title,
        grade,
        areas: areas_summary,
        areas_list,
        cancelled,
        issued_ms,
        source: "jian".into(),
        source_priority: 3,
    })
}

fn jian_tsunami_grade(_agency: &str, level: &str, title: &str) -> String {
    let s = format!("{level} {title}");
    if s.contains("大津波") || s.contains("大海啸") {
        return "MajorWarning".into();
    }
    if s.contains("津波警報")
        || s.contains("海啸警报")
        || level.contains("警報")
        || level.contains("警报")
    {
        return "Warning".into();
    }
    if s.contains("注意") || s.contains("Watch") || s.contains("Advisory") {
        return "Watch".into();
    }
    if s.contains("Warning") {
        return "Warning".into();
    }
    "Forecast".into()
}

fn parse_jian_tsunami_areas(agency: &str, area: &str, fallback_grade: &str) -> Vec<TsunamiArea> {
    if !agency.starts_with("jma") || area.is_empty() || area == "—" {
        return Vec::new();
    }
    let mut out: Vec<TsunamiArea> = Vec::new();
    for part in area.split(['；', ';', '\n']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let clean = part.trim_start_matches(['•', '・', '-', ' ']);
        let g = if clean.contains("大津波") {
            "MajorWarning"
        } else if clean.contains("津波警報") || clean.contains("警報") {
            "Warning"
        } else if clean.contains("注意") {
            "Watch"
        } else if clean.contains("予報") {
            "Forecast"
        } else {
            fallback_grade
        };
        let name = clean
            .split(['[', ' ', '\t'])
            .next()
            .unwrap_or(clean)
            .trim()
            .to_string();
        if name.len() < 2 {
            continue;
        }
        if !out.iter().any(|a| a.name == name) {
            out.push(TsunamiArea {
                name,
                grade: g.into(),
            });
        }
    }
    out
}

fn is_eew_agency(agency: &str) -> bool {
    let a = agency.trim().to_ascii_lowercase().replace('_', "-");
    matches!(
        a.as_str(),
        "jma-eew"
            | "cea"
            | "cea-pr"
            | "cwa-eew"
            | "sa"
            | "kma-eew"
            | "early-est"
            | "earlyest"
    ) || a.ends_with("-eew")
        || a.starts_with("cea-pr:")
        || a.starts_with("cea-pr/")
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
        .or_else(|| data.get("place"))
        .or_else(|| data.get("epicenter"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let event_id = json_id_string(data, &["id", "EventID", "eventId"]).unwrap_or_else(|| {
        let lat = field_f64(data, &["latitude", "Latitude"]).unwrap_or(0.0);
        let lon = field_f64(data, &["longitude", "Longitude"]).unwrap_or(0.0);
        let t = data
            .get("originTime")
            .or_else(|| data.get("shockTime"))
            .map(parse_origin_ms)
            .unwrap_or(0);
        format!("anon:{t}:{lat:.3}:{lon:.3}")
    });
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
    let is_auto = cenc_is_auto(agency, data);

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
        is_auto,
    })
}

/// CENC 自动测定：Wolfx/Jian `type=automatic`，或文案含「自动」。
fn cenc_is_auto(agency: &str, data: &Value) -> bool {
    let a = agency.to_ascii_lowercase();
    if !(a.contains("cenc") || a == "beijing") {
        return false;
    }
    if let Some(t) = data
        .get("type")
        .or_else(|| data.get("Type"))
        .or_else(|| data.get("infoType"))
        .and_then(|v| v.as_str())
    {
        let t = t.trim().to_ascii_lowercase();
        if t == "automatic" || t == "auto" || t.contains("自动") {
            return true;
        }
        if t == "reviewed" || t.contains("正式") {
            return false;
        }
    }
    false
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
        let is_auto = cenc_is_auto("wolfx-cenc", item);

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
            is_auto,
        }));
    }
}

/// P2PQuake JSON API v2：`code` 551 地震情报；552 海啸情报。
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
        if code == 552 {
            if let Some(t) = parse_p2p_tsunami(item) {
                let _ = tx.send(NetEvent::Tsunami(t));
            }
            continue;
        }
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
            is_auto: false,
        }));
    }
}

/// P2PQuake 552：areas[].grade 常见 MajorWarning / Warning / Watch / Forecast。
fn parse_p2p_tsunami(item: &Value) -> Option<TsunamiInfo> {
    let cancelled = item
        .get("cancelled")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let areas = item.get("areas").and_then(|a| a.as_array());
    let mut grade = "Forecast".to_string();
    let mut names: Vec<String> = Vec::new();
    let mut best_rank = 0_u8;
    if let Some(arr) = areas {
        for a in arr {
            let g = a
                .get("grade")
                .and_then(|x| x.as_str())
                .unwrap_or("Forecast");
            let rank = match g {
                "MajorWarning" | "Major" => 4,
                "Warning" => 3,
                "Watch" => 2,
                "Forecast" => 1,
                _ => 0,
            };
            if rank >= best_rank {
                best_rank = rank;
                grade = g.to_string();
            }
            if let Some(n) = a.get("name").and_then(|x| x.as_str()) {
                if !n.is_empty() && !names.iter().any(|x| x == n) {
                    names.push(n.to_string());
                }
            }
        }
    }
    let title = if cancelled {
        "海啸情报解除".into()
    } else {
        match grade.as_str() {
            "MajorWarning" | "Major" => "大海啸警报".into(),
            "Warning" => "海啸警报".into(),
            "Watch" => "海啸注意报".into(),
            _ => "海啸预报".into(),
        }
    };
    let areas_summary = if names.is_empty() {
        "—".into()
    } else {
        names.into_iter().take(8).collect::<Vec<_>>().join("、")
    };
    let issued_ms = item
        .get("time")
        .and_then(|x| x.as_str())
        .and_then(parse_time_str)
        .or_else(|| {
            item.pointer("/issue/time")
                .and_then(|x| x.as_str())
                .and_then(parse_time_str)
        })
        .unwrap_or(0);
    Some(TsunamiInfo {
        title,
        grade: if cancelled {
            "Cancel".into()
        } else {
            grade
        },
        areas: areas_summary,
        areas_list: {
            let mut list = Vec::new();
            if let Some(arr) = areas {
                for a in arr {
                    let g = a
                        .get("grade")
                        .and_then(|x| x.as_str())
                        .unwrap_or("Forecast");
                    if let Some(n) = a.get("name").and_then(|x| x.as_str()) {
                        if !n.is_empty() {
                            list.push(TsunamiArea {
                                name: n.to_string(),
                                grade: g.to_string(),
                            });
                        }
                    }
                }
            }
            list
        },
        cancelled,
        issued_ms,
        source: "p2p".into(),
        source_priority: 1,
    })
}

/// PancakesAPI RealtimeEvent：`{ source, type, action, timestampMs, payload }`。
pub fn ingest_pancakes_event(tx: &NetTx, text: &str, cfg: &jian_config::PancakesSourceConfig) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let source = v.get("source").and_then(|x| x.as_str()).unwrap_or("");
    if source.is_empty() {
        return;
    }
    if !pancakes_source_enabled(cfg, source) {
        return;
    }
    let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("update");
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    let payload = v.get("payload").cloned().unwrap_or(v.clone());

    match source {
        "gq" => ingest_pancakes_gq(tx, action, &payload, cfg.gq_min_magnitude),
        "usgs" => ingest_pancakes_usgs(tx, action, &payload),
        "jma_eew" => ingest_pancakes_jma_eew(tx, action, &payload),
        "jma_eqlist" => ingest_pancakes_jma_eqlist(tx, action, &payload),
        // NMEFC 海啸/海浪等为报文类，不进 JMA 海啸顶条与沿岸绘制
        "nmefc" => {}
        "jma_tsunami" => ingest_pancakes_jma_tsunami(tx, &payload),
        "nmefc-wave" | "nmefc-surge" => ingest_pancakes_ocean_alert(tx, source, &payload),
        "cma" => ingest_pancakes_cma(tx, &payload),
        "jma_volcano" => ingest_pancakes_volcano(tx, &payload),
        _ => {
            if typ == "earthquake" {
                ingest_pancakes_usgs(tx, action, &payload);
            }
        }
    }
}

fn pancakes_source_enabled(cfg: &jian_config::PancakesSourceConfig, source: &str) -> bool {
    match source {
        "gq" => cfg.gq,
        "usgs" => cfg.usgs,
        "jma_eew" => cfg.jma_eew,
        "jma_eqlist" => cfg.jma_eqlist,
        "nmefc" => cfg.nmefc,
        "nmefc-wave" => cfg.nmefc_wave,
        "nmefc-surge" => cfg.nmefc_surge,
        "jma_tsunami" => cfg.jma_tsunami,
        "cma" => cfg.cma,
        "jma_volcano" => cfg.jma_volcano,
        _ => true,
    }
}


fn json_id_string(data: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        let Some(v) = data.get(*k) else {
            continue;
        };
        if let Some(s) = v.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(n) = v.as_u64() {
            return Some(n.to_string());
        }
        if let Some(n) = v.as_i64() {
            return Some(n.to_string());
        }
        if let Some(n) = v.as_f64() {
            if n.is_finite() {
                return Some(format!("{n:.0}"));
            }
        }
    }
    None
}

fn pancakes_id(payload: &Value, keys: &[&str]) -> String {
    let mut weak_numeric: Option<String> = None;
    for k in keys {
        let Some(v) = payload.get(*k) else {
            continue;
        };
        if let Some(s) = v.as_str() {
            if !s.is_empty() {
                return s.to_string();
            }
        }
        // 小数/报数级整数常是 revision，不能当 event 主键
        if let Some(n) = v.as_u64() {
            if n >= 100_000 {
                return n.to_string();
            }
            weak_numeric.get_or_insert_with(|| n.to_string());
            continue;
        }
        if let Some(n) = v.as_i64() {
            if n >= 100_000 {
                return n.to_string();
            }
            if n > 0 {
                weak_numeric.get_or_insert_with(|| n.to_string());
            }
            continue;
        }
        if let Some(n) = v.as_f64() {
            if n.is_finite() && n.abs() >= 100_000.0 {
                return format!("{n:.0}");
            }
        }
    }
    // 勿回落成固定 "unknown"，否则同源多震会互相顶掉
    let lat = field_f64(payload, &["latitude", "Latitude"]).unwrap_or(0.0);
    let lon = field_f64(payload, &["longitude", "Longitude"]).unwrap_or(0.0);
    let t = pancakes_origin_ms(payload);
    if lat.abs() > 0.01 || lon.abs() > 0.01 || t > 0 {
        return format!("anon:{t}:{lat:.3}:{lon:.3}");
    }
    weak_numeric.unwrap_or_else(|| "unknown".into())
}

fn pancakes_origin_ms(payload: &Value) -> i64 {
    if let Some(n) = payload
        .get("originTimeMs")
        .and_then(|x| x.as_i64())
        .or_else(|| payload.get("originTimeMs").and_then(|x| x.as_u64()).map(|u| u as i64))
    {
        return n;
    }
    if let Some(s) = payload
        .get("originTimeIso")
        .or_else(|| payload.get("originTime"))
        .or_else(|| payload.get("OriginTime"))
        .or_else(|| payload.get("issuedAt"))
        .or_else(|| payload.get("issueTime"))
        .or_else(|| payload.get("effective"))
        .and_then(|x| x.as_str())
    {
        if let Some(ms) = parse_time_str(s) {
            return ms;
        }
    }
    if let Some(n) = payload.get("originTime").and_then(|x| x.as_i64()) {
        return n;
    }
    0
}

fn ingest_pancakes_gq(tx: &NetTx, action: &str, payload: &Value, min_mag: f64) {
    // 优先 eventId：上游 id 常为报数/修订号，作主键会把同源多震互相顶掉
    let id = pancakes_id(payload, &["eventId", "EventID", "id"]);
    if action == "cancelled" || action == "canceled" {
        let _ = tx.send(NetEvent::Eew(EewReport {
            agency: AgencyId("gq".into()),
            event_id: id,
            serial: u32::MAX,
            place: "—".into(),
            magnitude: 0.0,
            depth_km: 0.0,
            latitude: 0.0,
            longitude: 0.0,
            origin_ms: 0,
            max_intensity_text: "—".into(),
            intensity_kind: IntensityKind::CnIntensity,
            intensity_level: 0,
            is_final: true,
            is_cancel: true,
        }));
        return;
    }
    let magnitude = field_f64(payload, &["magnitude"]).unwrap_or(0.0);
    if min_mag > 0.0 && magnitude + 1e-9 < min_mag {
        return;
    }
    let latitude = field_f64(payload, &["latitude"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["longitude"]).unwrap_or(0.0);
    let depth_km = field_f64(payload, &["depth", "depthKm"]).unwrap_or(10.0);
    let place = payload
        .get("region")
        .or_else(|| payload.get("place"))
        .and_then(|x| x.as_str())
        .unwrap_or("GlobalQuake")
        .to_string();
    // GQ 报数 = revisionId（修订版本号）；updates / reportNum 作兼容回退
    let serial = payload
        .get("revisionId")
        .or_else(|| payload.get("updates"))
        .or_else(|| payload.get("reportNum"))
        .or_else(|| payload.get("revision"))
        .and_then(|x| x.as_u64().or_else(|| x.as_i64().map(|i| i as u64)))
        .map(|n| n as u32)
        .filter(|&n| n > 0 && n < 10_000)
        .unwrap_or(1);
    let raw = json_intensity_raw(payload, &["intensity", "maxIntensity"]);
    // GQ 无上游烈度时不估算（避免 M1 级被估成烈度 2）
    let (max_intensity_text, intensity_level) = match raw.as_deref() {
        Some(s) if !s.is_empty() && s != "—" && s != "-" && s != "null" => {
            fill_intensity(IntensityKind::CnIntensity, magnitude, depth_km, Some(s))
        }
        _ => ("—".into(), 0),
    };
    let archived = action == "archived"
        || payload
            .get("status")
            .and_then(|x| x.as_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("archived"));
    let _ = tx.send(NetEvent::Eew(EewReport {
        agency: AgencyId("gq".into()),
        event_id: id,
        serial,
        place: crate::place::fix_place("gq", &place, latitude, longitude),
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms: pancakes_origin_ms(payload),
        max_intensity_text,
        intensity_kind: IntensityKind::CnIntensity,
        intensity_level,
        is_final: archived,
        is_cancel: false,
    }));
}

fn ingest_pancakes_usgs(tx: &NetTx, action: &str, payload: &Value) {
    if action == "cancelled" || action == "canceled" {
        return;
    }
    let place = payload
        .get("placeName")
        .or_else(|| payload.get("place"))
        .and_then(|x| x.as_str())
        .unwrap_or("—")
        .to_string();
    let magnitude = field_f64(payload, &["magnitude"]).unwrap_or(0.0);
    let latitude = field_f64(payload, &["latitude"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["longitude"]).unwrap_or(0.0);
    let depth_km = field_f64(payload, &["depth", "depthKm"]).unwrap_or(10.0);
    let origin_ms = pancakes_origin_ms(payload);
    let raw = json_intensity_raw(payload, &["maxIntensity", "intensity"]);
    let (intensity_text, intensity_level) =
        fill_intensity(IntensityKind::CnIntensity, magnitude, depth_km, raw.as_deref());
    let info = payload
        .get("infoType")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let is_auto = info.eq_ignore_ascii_case("Automatic") || info.eq_ignore_ascii_case("auto");
    let _ = tx.send(NetEvent::Record(EqRecord {
        agency: AgencyId("usgs".into()),
        place: crate::place::fix_place("usgs", &place, latitude, longitude),
        time_text: format_time_text(origin_ms),
        magnitude,
        intensity_text,
        intensity_kind: IntensityKind::CnIntensity,
        intensity_level,
        latitude,
        longitude,
        depth_km,
        origin_ms,
        is_auto,
    }));
}

fn ingest_pancakes_jma_eew(tx: &NetTx, action: &str, payload: &Value) {
    let event_id = pancakes_id(payload, &["EventID", "eventId", "id"]);
    if action == "cancelled"
        || action == "canceled"
        || flag_true(payload, &["isCancel", "isCanceled", "isCancelled"])
    {
        let _ = tx.send(NetEvent::Eew(EewReport {
            agency: AgencyId("jma-eew".into()),
            event_id,
            serial: u32::MAX,
            place: "—".into(),
            magnitude: 0.0,
            depth_km: 0.0,
            latitude: 0.0,
            longitude: 0.0,
            origin_ms: 0,
            max_intensity_text: "—".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 0,
            is_final: true,
            is_cancel: true,
        }));
        return;
    }
    // 摘要项可能只有扁平字段
    if let Some(eew) = parse_jian_eew("jma-eew", payload) {
        let _ = tx.send(NetEvent::Eew(eew));
        return;
    }
    let place = payload
        .get("Hypocenter")
        .or_else(|| payload.get("place"))
        .or_else(|| payload.get("placeName"))
        .and_then(|x| x.as_str())
        .unwrap_or("—")
        .to_string();
    let magnitude = field_f64(payload, &["Magunitude", "Magnitude", "magnitude"]).unwrap_or(0.0);
    let depth_km = field_f64(payload, &["Depth", "depth", "depthKm"]).unwrap_or(10.0);
    let latitude = field_f64(payload, &["Latitude", "latitude"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["Longitude", "longitude"]).unwrap_or(0.0);
    let serial = payload
        .get("Serial")
        .or_else(|| payload.get("serial"))
        .or_else(|| payload.get("revision"))
        .or_else(|| payload.get("number"))
        .and_then(|x| x.as_u64().or_else(|| x.as_i64().map(|i| i as u64)))
        .unwrap_or(1) as u32;
    let raw = json_intensity_raw(payload, &["MaxIntensity", "maxIntensity", "intensity"]);
    let (max_intensity_text, intensity_level) =
        fill_intensity(IntensityKind::JmaShindo, magnitude, depth_km, raw.as_deref());
    let status = payload
        .get("status")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let is_cancel = status.eq_ignore_ascii_case("cancelled")
        || status.eq_ignore_ascii_case("canceled")
        || flag_true(payload, &["isCancel", "isCanceled", "isCancelled"]);
    let _ = tx.send(NetEvent::Eew(EewReport {
        agency: AgencyId("jma-eew".into()),
        event_id,
        serial: serial.max(1),
        place: crate::place::fix_place("jma-eew", &place, latitude, longitude),
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms: pancakes_origin_ms(payload),
        max_intensity_text,
        intensity_kind: IntensityKind::JmaShindo,
        intensity_level,
        is_final: flag_true(payload, &["isFinal", "is_final"])
            || status.eq_ignore_ascii_case("final"),
        is_cancel,
    }));
}

fn ingest_pancakes_jma_eqlist(tx: &NetTx, action: &str, payload: &Value) {
    if action == "cancelled" || action == "canceled" {
        return;
    }
    let place = payload
        .get("placeName")
        .or_else(|| payload.get("place"))
        .and_then(|x| x.as_str())
        .unwrap_or("—")
        .to_string();
    let magnitude = field_f64(payload, &["magnitude"]).unwrap_or(0.0);
    let latitude = field_f64(payload, &["latitude"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["longitude"]).unwrap_or(0.0);
    let depth_km = field_f64(payload, &["depth", "depthKm"]).unwrap_or(10.0);
    let origin_ms = pancakes_origin_ms(payload);
    let raw = json_intensity_raw(payload, &["maxIntensity", "intensity"]);
    let (intensity_text, intensity_level) =
        fill_intensity(IntensityKind::JmaShindo, magnitude, depth_km, raw.as_deref());
    let _ = tx.send(NetEvent::Record(EqRecord {
        agency: AgencyId("jma".into()),
        place: crate::place::fix_place("jma", &place, latitude, longitude),
        time_text: format_time_text(origin_ms),
        magnitude,
        intensity_text,
        intensity_kind: IntensityKind::JmaShindo,
        intensity_level,
        latitude,
        longitude,
        depth_km,
        origin_ms,
        is_auto: false,
    }));
}

fn ingest_pancakes_jma_tsunami(tx: &NetTx, payload: &Value) {
    let title = payload
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("津波予報")
        .to_string();
    let areas = payload.get("areas").and_then(|a| a.as_array());
    let mut grade = "Forecast".to_string();
    let mut names: Vec<String> = Vec::new();
    let mut best = 0_u8;
    if let Some(arr) = areas {
        for a in arr {
            let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("");
            let rank = if kind.contains("大津波") {
                4
            } else if kind.contains("津波警報") || kind == "警報" {
                3
            } else if kind.contains("注意") {
                2
            } else if kind.contains("予報") {
                1
            } else {
                0
            };
            if rank >= best {
                best = rank;
                grade = match rank {
                    4 => "MajorWarning",
                    3 => "Warning",
                    2 => "Watch",
                    _ => "Forecast",
                }
                .into();
            }
            if let Some(n) = a.get("name").and_then(|x| x.as_str()) {
                if !n.is_empty() && !names.iter().any(|x| x == n) {
                    names.push(n.to_string());
                }
            }
        }
    }
    // 摘要仅有 title
    if areas.is_none() {
        if title.contains("大津波") {
            grade = "MajorWarning".into();
        } else if title.contains("津波警報") || title.contains("警報") {
            grade = "Warning".into();
        } else if title.contains("注意") {
            grade = "Watch".into();
        } else {
            grade = "Forecast".into();
        }
    }
    let info_type = payload
        .get("infoType")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let cancelled = info_type.contains("取消") || info_type.contains("解除") || title.contains("解除");
    let zh_title = if cancelled {
        "海啸情报解除".into()
    } else {
        match grade.as_str() {
            "MajorWarning" => "大海啸警报".into(),
            "Warning" => "海啸警报".into(),
            "Watch" => "海啸注意报".into(),
            _ => "海啸预报".into(),
        }
    };
    let areas_summary = if names.is_empty() {
        "—".into()
    } else {
        names.into_iter().take(8).collect::<Vec<_>>().join("\u{3001}")
    };
    let issued_ms = payload
        .get("announcedTime")
        .or_else(|| payload.get("reportTime"))
        .or_else(|| payload.get("issuedAt"))
        .and_then(|x| x.as_str())
        .and_then(parse_time_str)
        .unwrap_or_else(|| pancakes_origin_ms(payload));
    let areas_list: Vec<TsunamiArea> = payload
        .get("areas")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let name = a.get("name").and_then(|x| x.as_str())?.to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                    let g = if kind.contains("大津波") {
                        "MajorWarning"
                    } else if kind.contains("津波警報") || kind == "警報" {
                        "Warning"
                    } else if kind.contains("注意") {
                        "Watch"
                    } else {
                        "Forecast"
                    };
                    Some(TsunamiArea {
                        name,
                        grade: g.into(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let _ = tx.send(NetEvent::Tsunami(TsunamiInfo {
        title: zh_title,
        grade: if cancelled {
            "Cancel".into()
        } else {
            grade
        },
        areas: areas_summary,
        areas_list,
        cancelled,
        issued_ms,
            source: "pancakes".into(),
        source_priority: 2,
}));
}

fn ingest_pancakes_ocean_alert(tx: &NetTx, source: &str, payload: &Value) {
    let title = payload
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or(if source == "nmefc-wave" {
            "海浪警报"
        } else {
            "风暴潮警报"
        })
        .to_string();
    let level = payload
        .get("level")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let agency = if source == "nmefc-wave" {
        "nmefc-wave"
    } else {
        "nmefc-surge"
    };
    let issued_ms = pancakes_origin_ms(payload);
    let _ = tx.send(NetEvent::Record(EqRecord {
        agency: AgencyId(agency.into()),
        place: format!("{title} ({level})"),
        time_text: format_time_text(issued_ms),
        magnitude: 0.0,
        intensity_text: level.into(),
        intensity_kind: IntensityKind::CnIntensity,
        intensity_level: match level {
            "红色" | "红" => 10,
            "橙色" | "橙" => 8,
            "黄色" | "黄" => 6,
            "蓝色" | "蓝" => 4,
            _ => 1,
        },
        latitude: 0.0,
        longitude: 0.0,
        depth_km: 0.0,
        origin_ms: issued_ms,
        is_auto: false,
    }));
}

fn ingest_pancakes_cma(tx: &NetTx, payload: &Value) {
    let title = payload
        .get("title")
        .or_else(|| payload.get("headline"))
        .and_then(|x| x.as_str())
        .unwrap_or("气象预警")
        .to_string();
    let latitude = field_f64(payload, &["latitude"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["longitude"]).unwrap_or(0.0);
    let issued_ms = pancakes_origin_ms(payload);
    let level = payload
        .get("type")
        .or_else(|| payload.get("level"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let _ = tx.send(NetEvent::Record(EqRecord {
        agency: AgencyId("cma".into()),
        place: crate::place::fix_place("cma", &title, latitude, longitude),
        time_text: format_time_text(issued_ms),
        magnitude: 0.0,
        intensity_text: level.into(),
        intensity_kind: IntensityKind::CnIntensity,
        intensity_level: 1,
        latitude,
        longitude,
        depth_km: 0.0,
        origin_ms: issued_ms,
        is_auto: false,
    }));
}

fn ingest_pancakes_volcano(tx: &NetTx, payload: &Value) {
    let title = payload
        .get("title")
        .or_else(|| payload.get("headline"))
        .and_then(|x| x.as_str())
        .unwrap_or("\u{706b}\u{5c71}\u{60c5}\u{62a5}")
        .to_string();
    let issued_ms = payload
        .get("announcedTime")
        .or_else(|| payload.get("issuedAt"))
        .and_then(|x| x.as_str())
        .and_then(parse_time_str)
        .unwrap_or_else(|| pancakes_origin_ms(payload));
    let name = payload
        .get("volcanoName")
        .or_else(|| payload.get("name"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            payload
                .get("areas")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|a| a.get("name"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| title.clone());
    let latitude = field_f64(payload, &["latitude", "Latitude", "lat"]).unwrap_or(0.0);
    let longitude = field_f64(payload, &["longitude", "Longitude", "lon", "lng"]).unwrap_or(0.0);
    let _ = tx.send(NetEvent::Volcano(jian_core::VolcanoNotice {
        agency: AgencyId("jma-volcano".into()),
        name,
        title,
        time_text: format_time_text(issued_ms),
        issued_ms,
        latitude,
        longitude,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::channel;

    #[test]
    fn list_response_agency_keeps_early_est() {
        assert_eq!(agency_from_list_response("early-estlist_response"), "early-est");
        assert_eq!(agency_from_list_response("jma-eewlist_response"), "jma-eew");
        assert_eq!(agency_from_list_response("cea-prlist_response"), "cea-pr");
        assert_eq!(agency_from_list_response("cenclist_response"), "cenc");
        assert_eq!(agency_from_list_response("jma_eew_list_response"), "jma_eew");
    }

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
        assert!(!rec.is_auto);
    }

    #[test]
    fn wolfx_cenc_automatic_flag() {
        let body = r#"{
            "No1": {
                "type": "automatic",
                "time": "2026-10-10 12:14:01",
                "placeName": "四川",
                "magnitude": "3.2",
                "depth": "10",
                "latitude": "30.0",
                "longitude": "103.0",
                "intensity": "3"
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_eqlist(&tx, body);
        let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
            panic!("expected Record");
        };
        assert!(rec.is_auto);
    }
}
