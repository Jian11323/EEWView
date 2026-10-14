//! 原始 JSON → core 模型（各源字段在此归一化）。

use jian_core::{
    agency_family, cea_pr_agency_id, cn_intensity_text, fill_intensity, instrumental_display_text,
    intensity_kind_for_agency, jma_from_instrumental, jma_shindo_level, jma_shindo_text, AgencyId,
    EewReport, EqRecord, IntensityKind, IntensityWarnArea, StationSample, TsunamiArea, TsunamiInfo,
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

/// 上游「调查中 / 不明」文案（JMA 震度速报等）。
fn is_investigating_text(s: &str) -> bool {
    let t = s.trim();
    matches!(
        t,
        "調査中"
            | "调查中"
            | "不明"
            | "?"
            | "なし"
            | "无"
            | "N/A"
            | "n/a"
            | "—"
            | "-"
            | "null"
    )
}

/// 字段值为调查中文案，或无法解析为数字。
fn field_looks_unknown(data: &Value, keys: &[&str]) -> bool {
    for k in keys {
        if let Some(v) = data.get(*k) {
            match v {
                Value::String(s) if is_investigating_text(s) => return true,
                Value::String(s) => {
                    let s = s.trim().trim_end_matches("km").trim_end_matches("KM").trim();
                    if !s.is_empty() && json_f64(v).is_none() {
                        return true;
                    }
                }
                Value::Number(n) => {
                    if n.as_f64().is_some_and(|f| f < 0.0) {
                        return true;
                    }
                }
                Value::Null => return true,
                _ => {}
            }
        }
    }
    false
}

/// 有震度原文时解析；震级调查中且无震度时不估算假烈度。
fn fill_record_intensity(
    kind: IntensityKind,
    magnitude: f64,
    depth_km: f64,
    raw: Option<&str>,
    magnitude_unknown: bool,
) -> (String, u8) {
    let raw_trim = raw.map(str::trim).unwrap_or("");
    if magnitude_unknown {
        if raw_trim.is_empty() || is_investigating_text(raw_trim) {
            let text = if raw_trim.is_empty() {
                "调查中".into()
            } else if raw_trim == "調査中" {
                "调查中".into()
            } else {
                raw_trim.into()
            };
            return (text, 0);
        }
        // 震度速报：最大震度已知，震级仍调查中
        return fill_intensity(kind, magnitude, depth_km, Some(raw_trim));
    }
    fill_intensity(kind, magnitude, depth_km, raw)
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

/// Jian `/s-net`：首包 `stations[]`；其后约每秒一帧 `time`+`timestamp`（对时）+ `int[]`。
/// `time` 为资料时刻（常按分钟对齐），`timestamp` 每秒递增，用于保活/对时，勿当僵死。
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
    for (i, s) in arr.iter().enumerate() {
        let mut code = s
            .get("code")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if code.is_empty() {
            code = format!("{i}");
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
/// `cfg` 为 `None` 时不按源开关过滤（单测）。
pub fn ingest_jian_frame(tx: &NetTx, text: &str, cfg: Option<&jian_config::JianSourceConfig>) {
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
        tracing::warn!(
            message = %jian_util::redact_secrets(msg),
            "jian: error 帧（常见原因：同令牌/同 IP 并发已满，白名单不增加并发额度）"
        );
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
                dispatch_jian_data(tx, agency, &data, cfg);
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
                dispatch_jian_data(tx, agency, item, cfg);
            }
        }
        return;
    }

    let data = v.get("Data").cloned().unwrap_or(v.clone());
    dispatch_jian_data(tx, typ, &data, cfg);
}

fn agency_from_source_key(k: &str) -> &str {
    // source：jma-eew / source:jma-eew / source：cea-pr:四川
    if let Some(rest) = k.strip_prefix("source：").or_else(|| k.strip_prefix("source:")) {
        return rest;
    }
    k.split('：').nth(1).or_else(|| k.split(':').nth(1)).unwrap_or(k)
}

fn dispatch_jian_data(
    tx: &NetTx,
    agency: &str,
    data: &Value,
    cfg: Option<&jian_config::JianSourceConfig>,
) {
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
        if let Some(c) = cfg {
            if !jian_accepts_eew(c, agency) {
                return;
            }
        }
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

fn jian_accepts_eew(cfg: &jian_config::JianSourceConfig, agency: &str) -> bool {
    use jian_core::AgencyFamily;
    match agency_family(agency) {
        AgencyFamily::Jma => cfg.jma_eew,
        AgencyFamily::Cea => cfg.cea_eew,
        AgencyFamily::Cwa => cfg.cwa_eew,
        AgencyFamily::Kma => cfg.kma_eew,
        AgencyFamily::Sa => cfg.sa_eew,
        AgencyFamily::EarlyEst => cfg.early_est_eew,
        _ => true,
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
    // 对齐 api/jma-tsunami.php：infoTypeName=発表/訂正/取消；cancel=整报取消；
    // cleared=至少一个 area[].cleared；area 为预报区对象数组。
    let info_type = data
        .get("infoTypeName")
        .or_else(|| data.get("infoType"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let title_raw = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let level = data.get("level").and_then(|v| v.as_str()).unwrap_or("");
    let cancel_flag = flag_true(data, &["cancel", "isCancel", "isCanceled", "isCancelled"])
        || info_type.contains("取消")
        || title_raw.contains("取消");
    let (areas_list, active_count) = parse_jian_tsunami_area_list(agency, data);
    let cleared_flag = flag_true(data, &["cleared"]);
    // 整报取消，或预报区全解除/失効 → 顶条清除
    let cancelled = cancel_flag
        || (cleared_flag && active_count == 0)
        || (!areas_list.is_empty() && active_count == 0)
        || level.contains("解除")
        || (title_raw.contains("解除") && active_count == 0);
    let grade = if cancelled {
        "Cancel".into()
    } else if !areas_list.is_empty() {
        jian_tsunami_grade_from_areas(&areas_list, level, &title_raw)
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
    let areas_summary = if areas_list.is_empty() {
        "—".into()
    } else {
        areas_list
            .iter()
            .take(8)
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("\u{3001}")
    };
    let issued_ms = data
        .get("originTime")
        .map(|v| {
            if let Some(n) = v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)) {
                normalize_epoch_ms(n)
            } else if let Some(n) = v.as_f64() {
                normalize_epoch_ms(n as i64)
            } else {
                parse_origin_ms(v)
            }
        })
        .unwrap_or(0);
    let expire_ms = parse_tsunami_expire_ms(data);
    Some(TsunamiInfo {
        title: zh_title,
        grade,
        areas: areas_summary,
        areas_list,
        cancelled,
        issued_ms,
        source: "jian".into(),
        source_priority: 3,
        expire_ms,
    })
}

fn normalize_epoch_ms(n: i64) -> i64 {
    if n <= 0 {
        0
    } else if n > 1_000_000_000_000 {
        n // already ms
    } else if n > 1_000_000_000 {
        n * 1000 // seconds
    } else {
        0
    }
}

fn parse_tsunami_expire_ms(data: &Value) -> i64 {
    for key in [
        "endTime",
        "expireTime",
        "expire_ms",
        "validUntil",
        "until",
        "cancelTime",
    ] {
        if let Some(v) = data.get(key) {
            if let Some(n) = v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)) {
                let ms = normalize_epoch_ms(n);
                if ms > 0 {
                    return ms;
                }
            }
            if let Some(n) = v.as_f64() {
                let ms = normalize_epoch_ms(n as i64);
                if ms > 0 {
                    return ms;
                }
            }
            if let Some(s) = v.as_str() {
                if let Some(ms) = parse_time_str(s) {
                    return ms;
                }
            }
        }
    }
    if let Some(desc) = data
        .get("description")
        .or_else(|| data.get("text"))
        .or_else(|| data.get("content"))
        .and_then(|x| x.as_str())
    {
        for marker in ["有效期至", "解除予定", "解除予定時刻", "まで有効"] {
            if let Some(idx) = desc.find(marker) {
                let digits: String = desc[idx..]
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .take(14)
                    .collect();
                if digits.len() >= 12 {
                    let padded = format!("{:0<14}", &digits[..digits.len().min(14)]);
                    if let Some(ms) = parse_time_str(&padded) {
                        return ms;
                    }
                }
            }
        }
    }
    0
}


fn jian_tsunami_grade(_agency: &str, level: &str, title: &str) -> String {
    tsunami_kind_to_grade(&format!("{level} {title}"))
}

fn jian_tsunami_grade_from_areas(areas: &[TsunamiArea], level: &str, title: &str) -> String {
    let mut best = 0_u8;
    let mut grade = "Forecast".to_string();
    for a in areas {
        let r = tsunami_grade_rank(&a.grade);
        if r > best {
            best = r;
            grade = a.grade.clone();
        }
    }
    if best == 0 {
        return jian_tsunami_grade("", level, title);
    }
    grade
}

fn tsunami_kind_to_grade(kind: &str) -> String {
    if kind.contains("大津波") || kind.contains("大海啸") {
        return "MajorWarning".into();
    }
    if kind.contains("津波警報")
        || kind.contains("海啸警报")
        || kind.contains("警報")
        || kind.contains("警报")
        || kind.contains("Warning")
    {
        return "Warning".into();
    }
    if kind.contains("注意") || kind.contains("Watch") || kind.contains("Advisory") {
        return "Watch".into();
    }
    "Forecast".into()
}

fn tsunami_grade_rank(grade: &str) -> u8 {
    match grade {
        "MajorWarning" | "Major" => 4,
        "Warning" => 3,
        "Watch" => 2,
        "Forecast" | "forecast" => 1,
        _ => 0,
    }
}

fn area_item_cleared(a: &Value) -> bool {
    if flag_true(a, &["cleared"]) {
        return true;
    }
    let kind = a.get("kind").and_then(|x| x.as_str()).unwrap_or("");
    let kind_code = a
        .get("kindCode")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    kind == "津波なし"
        || kind_code == "00"
        || kind.contains("解除")
        || kind.contains("切替")
        || kind.contains("失効")
}

/// 解析 `area[]` 对象数组（新格式）；兼容旧版字符串 `area`。
/// 返回 (未解除区列表, 未解除区数量)。
fn parse_jian_tsunami_area_list(agency: &str, data: &Value) -> (Vec<TsunamiArea>, usize) {
    let a = agency.trim().to_ascii_lowercase();
    if !(a.starts_with("jma") || a.contains("jma-tsunami") || a.contains("jma_tsunami")) {
        return (Vec::new(), 0);
    }
    if let Some(arr) = data.get("area").and_then(|v| v.as_array()) {
        let mut out: Vec<TsunamiArea> = Vec::new();
        let mut active = 0usize;
        for item in arr {
            let name = item
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if name.len() < 2 {
                continue;
            }
            if area_item_cleared(item) {
                continue;
            }
            let kind = item.get("kind").and_then(|x| x.as_str()).unwrap_or("");
            let grade = tsunami_kind_to_grade(kind);
            active += 1;
            if !out.iter().any(|x| x.name == name) {
                out.push(TsunamiArea { name, grade });
            }
        }
        return (out, active);
    }
    // 旧缓存：area / description 为顿号或分号分隔文本
    let area = data
        .get("area")
        .or_else(|| data.get("description"))
        .or_else(|| data.get("place"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if area.is_empty() || area == "—" {
        return (Vec::new(), 0);
    }
    let fallback = jian_tsunami_grade(
        agency,
        data.get("level").and_then(|v| v.as_str()).unwrap_or(""),
        data.get("title").and_then(|v| v.as_str()).unwrap_or(""),
    );
    let mut out: Vec<TsunamiArea> = Vec::new();
    for part in area.split(['；', ';', '\n']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let clean = part.trim_start_matches(['•', '・', '-', ' ']);
        let g = tsunami_kind_to_grade(clean);
        let grade = if tsunami_grade_rank(&g) > 0 {
            g
        } else {
            fallback.clone()
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
                grade,
            });
        }
    }
    let n = out.len();
    (out, n)
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
    // 对齐 api/jma-eew.php：isTraining 丢弃；isPLUM→假定震源；取消报允许缺经纬/震级。
    if flag_true(data, &["isTraining", "is_training", "training"]) {
        return None;
    }
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
    let is_cancel = flag_true(
        data,
        &["isCancel", "isCanceled", "isCancelled", "canceled", "cancelled", "is_cancel"],
    );
    let magnitude = match field_f64(data, &["magnitude", "Magunitude", "Magnitude"]) {
        Some(m) => m,
        None if is_cancel => 0.0,
        None => return None,
    };
    let depth_km = match field_f64(data, &["depth", "Depth"]) {
        Some(v) => v,
        None if is_cancel => 0.0,
        None => return None,
    };
    let latitude = match field_f64(data, &["latitude", "Latitude"]) {
        Some(v) => v,
        None if is_cancel => 0.0,
        None => return None,
    };
    let longitude = match field_f64(data, &["longitude", "Longitude"]) {
        Some(v) => v,
        None if is_cancel => 0.0,
        None => return None,
    };
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
    // lgIntensity：有则附在震度文案后（长周期阶级）
    let lg = data
        .get("lgIntensity")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "0" && s != "不明");
    let (mut max_intensity_text, intensity_level) =
        fill_intensity(kind, magnitude, depth_km, raw.as_deref());
    if let Some(lg) = lg {
        if max_intensity_text.is_empty() || max_intensity_text == "—" {
            max_intensity_text = format!("L{lg}");
        } else {
            max_intensity_text = format!("{max_intensity_text}/L{lg}");
        }
    }
    let is_assumption = eew_is_assumption(data, &place);
    let is_warn = eew_is_warn(data, intensity_level, kind);
    let warn_areas = if matches!(kind, IntensityKind::JmaShindo) && !is_cancel {
        parse_jma_warn_areas(data)
    } else {
        Vec::new()
    };

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
        is_final: flag_true(data, &["isFinal", "is_final", "final", "isLast", "isLastInfo"]),
        is_cancel,
        is_assumption,
        is_warn,
        final_at_ms: 0,
        warn_areas,
    })
}

/// 解析 JMA 官方预报分区（Wolfx `WarnArea` / Jian `warnArea`）。
/// 仅保留有区域名且能解析出震度档的项；缺字段不补假值。
fn parse_jma_warn_areas(data: &Value) -> Vec<IntensityWarnArea> {
    let raw = data
        .get("WarnArea")
        .or_else(|| data.get("warnArea"))
        .or_else(|| data.get("warn_area"));
    let Some(raw) = raw else {
        return Vec::new();
    };
    let parsed_string;
    let arr: &[Value] = match raw {
        Value::Array(a) => a.as_slice(),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() || t == "[]" {
                return Vec::new();
            }
            parsed_string = serde_json::from_str::<Value>(t).ok();
            match parsed_string.as_ref() {
                Some(Value::Array(a)) => a.as_slice(),
                _ => return Vec::new(),
            }
        }
        _ => return Vec::new(),
    };
    let mut best: std::collections::HashMap<String, IntensityWarnArea> =
        std::collections::HashMap::new();
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let name = obj
            .get("Chiiki")
            .or_else(|| obj.get("name"))
            .or_else(|| obj.get("Name"))
            .or_else(|| obj.get("addr"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(name) = name else {
            continue;
        };
        let intensity_raw = obj
            .get("Shindo1")
            .or_else(|| obj.get("intensity"))
            .or_else(|| obj.get("Intensity"))
            .or_else(|| obj.get("shindo"))
            .or_else(|| obj.get("maxIntensity"));
        let intensity_text = match intensity_raw {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(Value::Number(n)) => n
                .as_i64()
                .map(|i| i.to_string())
                .or_else(|| n.as_f64().map(|f| format!("{f}")))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let intensity_text = intensity_text.trim().to_string();
        if intensity_text.is_empty()
            || intensity_text == "?"
            || intensity_text == "不明"
            || intensity_text == "なし"
            || intensity_text == "无"
        {
            continue;
        }
        let level = jma_shindo_level(&intensity_text);
        let explicit_zero = {
            let t = intensity_text
                .replace('弱', "-")
                .replace('強', "+")
                .replace('强', "+");
            matches!(t.as_str(), "0" | "０") || t.parse::<i32>().ok() == Some(0)
        };
        if level == 0 && !explicit_zero {
            continue;
        }
        let display = jma_shindo_text(level).to_string();
        let area = IntensityWarnArea {
            name: name.clone(),
            intensity_text: display,
            level,
        };
        match best.get(&name) {
            Some(prev) if prev.level >= level => {}
            _ => {
                best.insert(name, area);
            }
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| b.level.cmp(&a.level).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Wolfx/JMA：仮定震源 / PLUM / 调查中。
fn eew_is_assumption(data: &Value, place: &str) -> bool {
    if flag_true(
        data,
        &[
            "isAssumption",
            "is_assumption",
            "assumption",
            "isAssumed",
            "isPLUM",
            "isPlum",
            "is_plum",
        ],
    ) {
        return true;
    }
    let hypo = data
        .get("Hypocenter")
        .or_else(|| data.get("hypocenter"))
        .or_else(|| data.get("placeName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    [place, hypo]
        .iter()
        .any(|s| s.contains("調査中") || s.contains("调查中") || s.contains("仮定"))
}

/// 警报：显式 isWarn，或标题含「警報」，或 JMA 震度≥5弱。
fn eew_is_warn(data: &Value, intensity_level: u8, intensity_kind: IntensityKind) -> bool {
    if flag_true(data, &["isWarn", "is_warn", "warn"]) {
        return true;
    }
    let title = data
        .get("Title")
        .or_else(|| data.get("title"))
        .or_else(|| data.get("infoTypeName"))
        .or_else(|| data.get("alertflg"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if title.contains("警報") || title.eq_ignore_ascii_case("Warning") {
        return true;
    }
    intensity_kind == IntensityKind::JmaShindo && intensity_level >= 5
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
    // 对齐 api/jma.php：infoType=取消 丢弃；震中/震级調査中时允许缺数（不补假值）。
    let info_type = data
        .get("infoType")
        .or_else(|| data.get("infoTypeName"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if info_type.contains("取消") {
        return None;
    }
    let place_raw = data
        .get("placeName")
        .or_else(|| data.get("location"))
        .or_else(|| data.get("place"))
        .or_else(|| data.get("Place"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let hypo_unknown = flag_true(data, &["hypocenterUnknown", "hypocenter_unknown"])
        || field_looks_unknown(data, &["latitude", "Latitude", "longitude", "Longitude"])
        || place_raw
            .as_deref()
            .is_some_and(|p| is_investigating_text(p) || p.contains("調査中") || p.contains("调查中"));
    let mag_unknown = flag_true(data, &["magnitudeUnknown", "magnitude_unknown"])
        || field_looks_unknown(data, &["magnitude", "Magnitude", "Magunitude"]);
    let magnitude = match field_f64(data, &["magnitude", "Magnitude", "Magunitude"]) {
        Some(m) if m >= 0.0 && !mag_unknown => m,
        Some(_) if mag_unknown => 0.0,
        None if mag_unknown || hypo_unknown => 0.0,
        _ => return None,
    };
    let latitude = match field_f64(data, &["latitude", "Latitude"]) {
        Some(v) if !hypo_unknown => v,
        _ if hypo_unknown => 0.0,
        _ => return None,
    };
    let longitude = match field_f64(data, &["longitude", "Longitude"]) {
        Some(v) if !hypo_unknown => v,
        _ if hypo_unknown => 0.0,
        _ => return None,
    };
    let depth_km = match field_f64(data, &["depth", "Depth"]) {
        Some(v) if v >= 0.0 && !hypo_unknown => v,
        _ if hypo_unknown => 0.0,
        Some(v) => v,
        None => return None,
    };
    let place = match place_raw {
        Some(p) if !is_investigating_text(&p) => p,
        _ if hypo_unknown => "调查中".into(),
        Some(p) => p,
        None => return None,
    };
    let origin_ms = data
        .get("originTime")
        .or_else(|| data.get("shockTime"))
        .or_else(|| data.get("time"))
        .or_else(|| data.get("Time"))
        .or_else(|| data.get("reportTime"))
        .map(parse_origin_ms)
        .unwrap_or(0);
    let kind = intensity_kind_for_agency(agency);
    let raw = json_intensity_raw(data, &["maxIntensity", "intensity", "MaxIntensity"]);
    let (intensity_text, intensity_level) =
        fill_record_intensity(kind, magnitude, depth_km, raw.as_deref(), mag_unknown);
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
        magnitude_unknown: mag_unknown,
        hypocenter_unknown: hypo_unknown,
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

/// Wolfx `all_eew` / 单源 WS：按 `type` 分发；各子源开关见 [`jian_config::WolfxSourceConfig`]。
pub fn ingest_wolfx_frame(tx: &NetTx, text: &str, cfg: &jian_config::WolfxSourceConfig) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "heartbeat" | "pong" => {}
        "jma_eew" if cfg.type_enabled("jma_eew") => emit_wolfx_eew(tx, &v, "wolfx-jma-eew"),
        "sc_eew" if cfg.type_enabled("sc_eew") => emit_wolfx_eew(tx, &v, "wolfx-cea-pr:四川"),
        "fj_eew" if cfg.type_enabled("fj_eew") => emit_wolfx_eew(tx, &v, "wolfx-cea-pr:福建"),
        "cq_eew" if cfg.type_enabled("cq_eew") => emit_wolfx_eew(tx, &v, "wolfx-cea-pr:重庆"),
        "cenc_eew" if cfg.type_enabled("cenc_eew") => emit_wolfx_eew(tx, &v, "wolfx-cea"),
        "cwa_eew" if cfg.type_enabled("cwa_eew") => emit_wolfx_eew(tx, &v, "wolfx-cwa-eew"),
        "cenc_eqlist" if cfg.type_enabled("cenc_eqlist") => ingest_wolfx_cenc_eqlist_value(tx, &v),
        "jma_eqlist" if cfg.type_enabled("jma_eqlist") => ingest_wolfx_jma_eqlist_value(tx, &v),
        // 专用 `cwa_eew` 通道可能无 type：HypoCenter + ReportNum（区别于 JMA 的 Hypocenter + Serial）
        "" if cfg.type_enabled("cwa_eew")
            && v.get("HypoCenter").is_some()
            && v.get("ReportNum").is_some() =>
        {
            emit_wolfx_eew(tx, &v, "wolfx-cwa-eew");
        }
        // 旧版单通道 jma_eew 可能省略 type
        "" if cfg.type_enabled("jma_eew")
            && v.get("Hypocenter").is_some()
            && v.get("Serial").is_some() =>
        {
            emit_wolfx_eew(tx, &v, "wolfx-jma-eew");
        }

        _ => {}
    }
}

/// 兼容旧调用：仅解析 `jma_eew`（或无 type 的 JMA 形扁平包）。
pub fn ingest_wolfx_jma_eew(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" {
        return;
    }
    if typ == "jma_eew" || typ.is_empty() {
        emit_wolfx_eew(tx, &v, "wolfx-jma-eew");
    }
}

/// 专用 `cwa_eew` WebSocket 文本帧。
pub fn ingest_wolfx_cwa_eew(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if typ == "heartbeat" || typ == "pong" {
        return;
    }
    emit_wolfx_eew(tx, &v, "wolfx-cwa-eew");
}

fn emit_wolfx_eew(tx: &NetTx, v: &Value, agency: &str) {
    // 心跳有时只有简短对象
    if v.get("EventID").is_none()
        && v.get("Hypocenter").is_none()
        && v.get("HypoCenter").is_none()
    {
        return;
    }
    let place = v
        .get("Hypocenter")
        .or_else(|| v.get("HypoCenter"))
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let event_id =
        json_id_string(v, &["EventID", "ID", "id"]).unwrap_or_else(|| "wolfx-unknown".into());
    let serial = v
        .get("Serial")
        .or_else(|| v.get("ReportNum"))
        .and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(1) as u32;
    // Magnitude 优先；Magunitude 仅为兼容旧拼写
    let Some(magnitude) = field_f64(v, &["Magnitude", "magnitude", "Magunitude"]) else {
        return;
    };
    let Some(depth_km) = field_f64(v, &["Depth", "depth"]) else {
        return;
    };
    let Some(latitude) = field_f64(v, &["Latitude", "latitude"]) else {
        return;
    };
    let Some(longitude) = field_f64(v, &["Longitude", "longitude"]) else {
        return;
    };
    let origin_ms = v
        .get("OriginTime")
        .or_else(|| v.get("originTime"))
        .and_then(|x| match x {
            Value::String(s) => parse_time_str(s),
            other => {
                let ms = parse_origin_ms(other);
                if ms > 0 {
                    Some(ms)
                } else {
                    None
                }
            }
        })
        .unwrap_or(0);

    let kind = intensity_kind_for_agency(agency);
    let raw = json_intensity_raw(v, &["MaxIntensity", "maxIntensity", "intensity"]);
    let (max_intensity_text, intensity_level) =
        fill_intensity(kind, magnitude, depth_km, raw.as_deref());

    let is_cancel = flag_true(v, &["isCancel", "isCanceled", "is_cancel"])
        || v.get("Type")
            .and_then(|x| x.as_str())
            .is_some_and(|s| s.contains("取消"));
    let is_final = !is_cancel
        && (flag_true(v, &["isFinal", "is_final", "Final"])
            || v.get("Type")
                .and_then(|x| x.as_str())
                .is_some_and(|s| s.contains("最終") || s.contains("最终")));

    let is_assumption = eew_is_assumption(v, &place);
    let is_warn = eew_is_warn(v, intensity_level, kind);
    let warn_areas = if matches!(kind, IntensityKind::JmaShindo) && !is_cancel {
        parse_jma_warn_areas(v)
    } else {
        Vec::new()
    };
    let _ = tx.send(NetEvent::Eew(EewReport {
        agency: AgencyId(agency.into()),
        event_id,
        serial,
        place: crate::place::fix_place(agency, &place, latitude, longitude),
        magnitude,
        depth_km,
        latitude,
        longitude,
        origin_ms,
        max_intensity_text,
        intensity_kind: kind,
        intensity_level,
        is_final,
        is_cancel,
        is_assumption,
        is_warn,
        final_at_ms: 0,
        warn_areas,
    }));
}

/// Wolfx `cenc_eqlist`（HTTP JSON 或 WS）：对象键为 No1..NoN 或数组。
/// 注意：官方字段 `magnitude` / `latitude` / `longitude` / `depth` / `intensity` 多为**字符串**。
pub fn ingest_wolfx_eqlist(tx: &NetTx, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    ingest_wolfx_cenc_eqlist_value(tx, &v);
}

fn wolfx_eqlist_items(v: &Value) -> Vec<&Value> {
    if let Some(arr) = v.as_array() {
        return arr.iter().collect();
    }
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let mut pairs: Vec<_> = obj
        .iter()
        .filter(|(k, _)| k.starts_with("No") || k.starts_with("no"))
        .collect();
    // No10 须排在 No2 之后：按数字后缀排序
    pairs.sort_by(|(a, _), (b, _)| {
        let na = a
            .trim_start_matches(['N', 'n', 'O', 'o'])
            .parse::<u32>()
            .unwrap_or(0);
        let nb = b
            .trim_start_matches(['N', 'n', 'O', 'o'])
            .parse::<u32>()
            .unwrap_or(0);
        na.cmp(&nb)
    });
    pairs.into_iter().map(|(_, val)| val).collect()
}

fn ingest_wolfx_cenc_eqlist_value(tx: &NetTx, v: &Value) {
    for item in wolfx_eqlist_items(v).into_iter().take(50) {
        let place = item
            .get("placeName")
            .or_else(|| item.get("location"))
            .or_else(|| item.get("place"))
            .or_else(|| item.get("Place"))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "—".into());
        let Some(magnitude) = field_f64(item, &["magnitude", "Magnitude"]) else {
            continue;
        };
        let Some(latitude) = field_f64(item, &["latitude", "Latitude"]) else {
            continue;
        };
        let Some(longitude) = field_f64(item, &["longitude", "Longitude"]) else {
            continue;
        };
        let Some(depth_km) = field_f64(item, &["depth", "Depth"]) else {
            continue;
        };
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

        let raw = json_intensity_raw(item, &["intensity", "maxIntensity", "MaxIntensity"]);
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
            magnitude_unknown: false,
            hypocenter_unknown: false,
        }));
    }
}

fn ingest_wolfx_jma_eqlist_value(tx: &NetTx, v: &Value) {
    for item in wolfx_eqlist_items(v).into_iter().take(50) {
        let place_raw = item
            .get("location")
            .or_else(|| item.get("place"))
            .or_else(|| item.get("Hypocenter"))
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mag_unknown = field_looks_unknown(item, &["magnitude", "Magnitude"]);
        let hypo_unknown = field_looks_unknown(item, &["latitude", "Latitude", "longitude", "Longitude"])
            || place_raw
                .as_deref()
                .is_some_and(|p| is_investigating_text(p) || p.contains("調査中") || p.contains("调查中"));
        let magnitude = match field_f64(item, &["magnitude", "Magnitude"]) {
            Some(m) if m >= 0.0 && !mag_unknown => m,
            _ if mag_unknown || hypo_unknown => 0.0,
            _ => continue,
        };
        let latitude = match field_f64(item, &["latitude", "Latitude"]) {
            Some(v) if !hypo_unknown => v,
            _ if hypo_unknown => 0.0,
            _ => continue,
        };
        let longitude = match field_f64(item, &["longitude", "Longitude"]) {
            Some(v) if !hypo_unknown => v,
            _ if hypo_unknown => 0.0,
            _ => continue,
        };
        let depth_km = match field_f64(item, &["depth", "Depth"]) {
            Some(v) if v >= 0.0 && !hypo_unknown => v,
            _ if hypo_unknown => 0.0,
            Some(v) => v,
            None => continue,
        };
        let place = match place_raw {
            Some(p) if !is_investigating_text(&p) => p,
            _ if hypo_unknown => "调查中".into(),
            Some(p) => p,
            None => "\u{2014}".into(),
        };
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
            .unwrap_or_else(|| "\u{2014}".into());

        let raw = json_intensity_raw(item, &["shindo", "MaxIntensity", "intensity"]);
        // 无震度且震级/震源皆调查中：仍入库（震度速报占位），不丢弃
        if mag_unknown
            && hypo_unknown
            && raw
                .as_deref()
                .map(|s| s.trim().is_empty() || is_investigating_text(s))
                .unwrap_or(true)
        {
            // 至少要有发震时刻，否则无展示价值
            if origin_ms <= 0 {
                continue;
            }
        }
        let (intensity_text, intensity_level) = fill_record_intensity(
            IntensityKind::JmaShindo,
            magnitude,
            depth_km,
            raw.as_deref(),
            mag_unknown,
        );

        let _ = tx.send(NetEvent::Record(EqRecord {
            agency: AgencyId("wolfx-jma".into()),
            place: crate::place::fix_place("wolfx-jma", &place, latitude, longitude),
            time_text,
            magnitude,
            intensity_text,
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level,
            latitude,
            longitude,
            depth_km,
            origin_ms,
            is_auto: false,
            magnitude_unknown: mag_unknown,
            hypocenter_unknown: hypo_unknown,
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
        // P2P：震源缺测 lat/lon=-200、depth=-1、magnitude=-1（震度速报等）
        let lat_raw = field_f64(hypo, &["latitude", "Latitude"]).unwrap_or(-200.0);
        let lon_raw = field_f64(hypo, &["longitude", "Longitude"]).unwrap_or(-200.0);
        let mag_raw = field_f64(hypo, &["magnitude", "Magnitude"]).unwrap_or(-1.0);
        let depth_raw = field_f64(hypo, &["depth", "Depth"]).unwrap_or(-1.0);
        let hypo_unknown = lat_raw <= -180.0 || lon_raw <= -180.0;
        let mag_unknown = mag_raw < 0.0;
        let depth_unknown = depth_raw < 0.0;
        let latitude = if hypo_unknown { 0.0 } else { lat_raw };
        let longitude = if hypo_unknown { 0.0 } else { lon_raw };
        let magnitude = if mag_unknown { 0.0 } else { mag_raw };
        let depth_km = if hypo_unknown || depth_unknown {
            0.0
        } else {
            depth_raw
        };
        let place_raw = hypo
            .get("name")
            .and_then(|x| x.as_str())
            .map(|s| s.trim())
            .unwrap_or("");
        let place = if place_raw.is_empty() || is_investigating_text(place_raw) {
            if hypo_unknown {
                "调查中"
            } else if place_raw.is_empty() {
                "\u{2014}"
            } else {
                place_raw
            }
        } else {
            place_raw
        };
        let max_scale = eq.get("maxScale").and_then(|x| x.as_i64());
        // 无震源、无震级、无有效震度：丢弃
        if hypo_unknown && mag_unknown && max_scale.filter(|&s| s > 0).is_none() {
            continue;
        }
        let raw_intensity = max_scale.and_then(|scale| {
            if scale <= 0 {
                None
            } else {
                Some(jian_core::jma_shindo_text(jma_shindo_level(&scale.to_string())).to_string())
            }
        });
        let (intensity_text, level) = fill_record_intensity(
            IntensityKind::JmaShindo,
            magnitude,
            depth_km,
            raw_intensity.as_deref(),
            mag_unknown,
        );
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
            .unwrap_or_else(|| "\u{2014}".into());

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
            magnitude_unknown: mag_unknown,
            hypocenter_unknown: hypo_unknown,
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
        expire_ms: 0,
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
    let payload = v.get("payload").cloned().unwrap_or(v.clone());

    match source {
        "gq" => ingest_pancakes_gq(tx, action, &payload, cfg.gq_min_magnitude),
        "jma_volcano" => ingest_pancakes_volcano(tx, &payload),
        _ => {}
    }
}

fn pancakes_source_enabled(cfg: &jian_config::PancakesSourceConfig, source: &str) -> bool {
    match source {
        "gq" => cfg.gq,
        "jma_volcano" => cfg.jma_volcano,
        _ => false,
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
        .or_else(|| payload.get("time"))
        .or_else(|| payload.get("Time"))
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
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
            warn_areas: Vec::new(),
        }));
        return;
    }
    let Some(magnitude) = field_f64(payload, &["magnitude"]) else {
        return;
    };
    if min_mag > 0.0 && magnitude + 1e-9 < min_mag {
        return;
    }
    let Some(latitude) = field_f64(payload, &["latitude"]) else {
        return;
    };
    let Some(longitude) = field_f64(payload, &["longitude"]) else {
        return;
    };
    let Some(depth_km) = field_f64(payload, &["depth", "depthKm"]) else {
        return;
    };
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
        is_assumption: false,
        is_warn: false,
        final_at_ms: 0,
            warn_areas: Vec::new(),
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
    let mut latitude = field_f64(payload, &["latitude", "Latitude", "lat"]).unwrap_or(0.0);
    let mut longitude = field_f64(payload, &["longitude", "Longitude", "lon", "lng"]).unwrap_or(0.0);
    if latitude.abs() <= 0.01 && longitude.abs() <= 0.01 {
        if let Some(arr) = payload.get("areas").and_then(|a| a.as_array()) {
            for a in arr {
                let la = field_f64(a, &["latitude", "Latitude", "lat"]).unwrap_or(0.0);
                let lo = field_f64(a, &["longitude", "Longitude", "lon", "lng"]).unwrap_or(0.0);
                if la.abs() > 0.01 || lo.abs() > 0.01 {
                    latitude = la;
                    longitude = lo;
                    break;
                }
            }
        }
    }
    if latitude.abs() <= 0.01 && longitude.abs() <= 0.01 {
        if let Some((la, lo)) = jma_volcano_coords(&name).or_else(|| jma_volcano_coords(&title)) {
            latitude = la;
            longitude = lo;
        }
    }
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

fn jma_volcano_coords(name: &str) -> Option<(f64, f64)> {
    const TABLE: &[(&str, f64, f64)] = &[
        ("桜島", 31.593, 130.657),
        ("薩摩硫黄島", 30.793, 130.305),
        ("口永良部島", 30.447, 130.217),
        ("諏訪之瀬島", 29.638, 129.716),
        ("阿蘇山", 32.884, 131.104),
        ("霧島山", 31.934, 130.862),
        ("新燃岳", 31.909, 130.886),
        ("十勝岳", 43.418, 142.686),
        ("樽前山", 42.690, 141.380),
        ("俱多楽", 42.488, 141.157),
        ("北海道駒ヶ岳", 42.063, 140.677),
        ("有珠山", 42.157, 140.412),
        ("白山", 36.135, 138.523),
        ("吾妻山", 37.647, 140.081),
        ("草津白根山", 36.650, 139.233),
        ("那須岳", 36.864, 138.114),
        ("富士山", 35.363, 138.731),
        ("伊豆大島", 34.724, 139.004),
        ("三宅島", 34.736, 139.395),
        ("大島", 34.722, 139.398),
        ("三原山", 34.217, 139.150),
        ("硫黄島", 30.793, 130.305),
        ("桜島", 31.593, 130.657),
    ];
    let n = name.trim();
    for (k, la, lo) in TABLE {
        if n.contains(k) {
            return Some((*la, *lo));
        }
    }
    None
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
        assert!(rec.origin_ms > 0, "CENC time must parse to origin_ms");
        // 2026-10-10 12:14:01 CST
        assert_eq!(rec.origin_ms, 1_791_605_641_000);
    }

    #[test]
    fn snet_accepts_1hz_time_sync_intensity_frame() {
        // 实况形状：首包 stations；其后每秒 time+timestamp+int（对时帧兼强度）
        let mut state = KmoniState::default();
        let (tx, mut rx) = channel();
        ingest_snet_frame(
            &mut state,
            &tx,
            r#"{"type":"s-net","timestamp":1,"stations":[{"code":"A","name":"a","lat":35.0,"lon":139.0},{"code":"B","name":"b","lat":36.0,"lon":140.0}],"source":"s-net"}"#,
        );
        assert!(rx.try_recv().is_err(), "meta-only must not emit Stations");
        ingest_snet_frame(
            &mut state,
            &tx,
            r#"{"type":"s-net","time":"20261014084800","timestamp":1791967763000,"int":[-1.3,-4.0],"source":"s-net"}"#,
        );
        let NetEvent::Stations { network, list } = rx.try_recv().expect("stations") else {
            panic!("expected Stations");
        };
        assert_eq!(network, "snet");
        // -1.3 有感档；-4.0 缺测跳过
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "A");
        assert!((list[0].instrumental - (-1.3)).abs() < 1e-9);
    }

    #[test]
    fn wolfx_cenc_skips_when_depth_missing() {
        let body = r#"{
            "No1": {
                "type": "reviewed",
                "time": "2026-10-10 12:14:01",
                "placeName": "巴拿马",
                "magnitude": "6.0",
                "latitude": "7.50",
                "longitude": "-80.60",
                "intensity": "8"
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_eqlist(&tx, body);
        assert!(rx.try_recv().is_err(), "missing depth must not invent 10km");
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

    #[test]
    fn wolfx_all_dispatches_sc_eew() {
        let body = r#"{
            "type": "sc_eew",
            "ID": 1,
            "EventID": "SC202610110001",
            "ReportTime": "2026-10-11 12:00:00",
            "ReportNum": 2,
            "OriginTime": "2026-10-11 11:59:30",
            "HypoCenter": "四川阿坝州红原县",
            "Latitude": 32.8,
            "Longitude": 102.6,
            "Magnitude": 4.5,
            "Depth": 12.0,
            "MaxIntensity": 5.0
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_frame(&tx, body, &jian_config::WolfxSourceConfig {
            enabled: true,
            eew_enabled: true,
            eqlist_enabled: true,
            jma_eew: true,
            sc_eew: true,
            fj_eew: true,
            cq_eew: true,
            cenc_eew: true,
            cwa_eew: true,
            cenc_eqlist: true,
            jma_eqlist: true,
            ws_urls: vec![],
            http_eqlist: String::new(),
        });
        let NetEvent::Eew(e) = rx.try_recv().expect("eew") else {
            panic!("expected Eew");
        };
        assert_eq!(e.agency.0, "wolfx-cea-pr:四川");
        assert_eq!(e.event_id, "SC202610110001");
        assert_eq!(e.serial, 2);
        assert!((e.magnitude - 4.5).abs() < 1e-9);
        assert_eq!(e.intensity_kind, IntensityKind::CnIntensity);
    }

    #[test]
    fn wolfx_all_dispatches_jma_eqlist() {
        let body = r#"{
            "type": "jma_eqlist",
            "No1": {
                "time": "2026/10/11 12:00:00",
                "location": "千葉県東方沖",
                "magnitude": "4.2",
                "shindo": "3",
                "depth": "40km",
                "latitude": "35.5",
                "longitude": "140.5"
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_frame(&tx, body, &jian_config::WolfxSourceConfig {
            enabled: true,
            eew_enabled: true,
            eqlist_enabled: true,
            jma_eew: true,
            sc_eew: true,
            fj_eew: true,
            cq_eew: true,
            cenc_eew: true,
            cwa_eew: true,
            cenc_eqlist: true,
            jma_eqlist: true,
            ws_urls: vec![],
            http_eqlist: String::new(),
        });
        let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
            panic!("expected Record");
        };
        assert_eq!(rec.agency.0, "wolfx-jma");
        assert!((rec.magnitude - 4.2).abs() < 1e-9);
        assert_eq!(rec.intensity_kind, IntensityKind::JmaShindo);
        assert!(rec.origin_ms > 0);
    }

    #[test]
    fn wolfx_eew_toggle_skips_when_disabled() {
        let body = r#"{
            "type": "jma_eew",
            "EventID": "20261011201300",
            "Serial": 1,
            "OriginTime": "2026/10/11 20:13:00",
            "Hypocenter": "未知",
            "Latitude": 35.0,
            "Longitude": 139.0,
            "Magnitude": 5.0,
            "Depth": 10.0,
            "MaxIntensity": "3"
        }"#;
        let (tx, mut rx) = channel();
        ingest_wolfx_frame(&tx, body, &jian_config::WolfxSourceConfig {
            enabled: true,
            eew_enabled: true,
            eqlist_enabled: true,
            jma_eew: false,
            sc_eew: true,
            fj_eew: true,
            cq_eew: true,
            cenc_eew: true,
            cwa_eew: true,
            cenc_eqlist: true,
            jma_eqlist: true,
            ws_urls: vec![],
            http_eqlist: String::new(),
        });
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn jma_warn_areas_from_wolfx_chiiki() {
        let v = serde_json::json!({
            "WarnArea": [
                {"Chiiki": "石狩地方北部", "Shindo1": "5弱"},
                {"Chiiki": "石狩地方北部", "Shindo1": "4"},
                {"Chiiki": "渡島地方北部", "Shindo1": "3"},
                {"Chiiki": "", "Shindo1": "5強"},
                {"Chiiki": "不明区", "Shindo1": "不明"}
            ]
        });
        let areas = parse_jma_warn_areas(&v);
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].name, "石狩地方北部");
        assert_eq!(areas[0].level, 5);
        assert_eq!(areas[0].intensity_text, "5弱");
        assert_eq!(areas[1].name, "渡島地方北部");
        assert_eq!(areas[1].level, 3);
    }

    #[test]
    fn jian_eew_skips_training_and_maps_plum() {
        let body = r#"{
            "type": "jma-eew",
            "Data": {
                "id": "202610120001",
                "originTime": "2026-10-12T08:00:00+09:00",
                "latitude": 35.0,
                "longitude": 139.0,
                "depth": 10.0,
                "magnitude": 4.0,
                "placeName": "東京湾",
                "infoTypeName": "予報",
                "intensity": "3",
                "serial": 1,
                "isWarn": false,
                "isFinal": false,
                "isCancel": false,
                "isTraining": true,
                "isPLUM": false,
                "warnArea": []
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_jian_frame(&tx, body, None);
        assert!(rx.try_recv().is_err());

        let body2 = body
            .replace("\"isTraining\": true", "\"isTraining\": false")
            .replace("\"isPLUM\": false", "\"isPLUM\": true");
        ingest_jian_frame(&tx, &body2, None);
        let NetEvent::Eew(e) = rx.try_recv().expect("eew") else {
            panic!("expected Eew");
        };
        assert!(e.is_assumption);
        assert_eq!(e.event_id, "202610120001");
    }

    #[test]
    fn jian_record_investigating_keeps_intensity() {
        let body = r#"{
            "type": "jma",
            "Data": {
                "id": "20261014120000",
                "originTime": "2026-10-14T12:00:00+09:00",
                "latitude": 0,
                "longitude": 0,
                "depth": 0,
                "magnitude": 0,
                "placeName": "調査中",
                "intensity": "4",
                "infoType": "発表",
                "hypocenterUnknown": true,
                "magnitudeUnknown": true
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_jian_frame(&tx, body, None);
        let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
            panic!("expected Record");
        };
        assert!(rec.magnitude_unknown);
        assert!(rec.hypocenter_unknown);
        assert!(!rec.has_map_coord());
        assert_eq!(rec.intensity_text, "4");
        assert_eq!(rec.intensity_level, 4);
        assert_eq!(rec.place, "调查中");
        assert_eq!(rec.magnitude_label(), "调查中");
    }

    #[test]
    fn p2p_scale_prompt_investigating_hypocenter() {
        let body = r#"[{
            "code": 551,
            "earthquake": {
                "time": "2026/10/14 12:00:00",
                "maxScale": 40,
                "hypocenter": {
                    "name": "",
                    "latitude": -200,
                    "longitude": -200,
                    "depth": -1,
                    "magnitude": -1
                }
            }
        }]"#;
        let (tx, mut rx) = channel();
        ingest_p2p_items(&tx, body);
        let NetEvent::Record(rec) = rx.try_recv().expect("record") else {
            panic!("expected Record");
        };
        assert!(rec.hypocenter_unknown);
        assert!(rec.magnitude_unknown);
        assert_eq!(rec.intensity_text, "4");
        assert_eq!(rec.place, "调查中");
    }

    #[test]
    fn jian_tsunami_parses_area_array_and_cancel() {
        let body = r#"{
            "type": "jma-tsunami",
            "Data": {
                "id": "20261010025608",
                "title": "津波警報",
                "level": "津波警報",
                "headline": "",
                "number": 1,
                "originTime": 1791574620000,
                "infoTypeName": "発表",
                "cancel": false,
                "cleared": false,
                "orgUnit": "日本気象庁",
                "telegram": "VTSE41",
                "area": [
                    {"code": "100", "name": "北海道太平洋沿岸東部", "kind": "津波警報", "kindCode": "52", "cleared": false},
                    {"code": "101", "name": "青森県太平洋沿岸", "kind": "津波なし", "kindCode": "00", "cleared": true}
                ]
            }
        }"#;
        let (tx, mut rx) = channel();
        ingest_jian_frame(&tx, body, None);
        let NetEvent::Tsunami(tsu) = rx.try_recv().expect("tsu") else {
            panic!("expected Tsunami");
        };
        assert!(!tsu.cancelled);
        assert_eq!(tsu.grade, "Warning");
        assert_eq!(tsu.areas_list.len(), 1);
        assert_eq!(tsu.areas_list[0].name, "北海道太平洋沿岸東部");

        let cancel = body
            .replace("\"cancel\": false", "\"cancel\": true")
            .replace("\"infoTypeName\": \"発表\"", "\"infoTypeName\": \"取消\"");
        ingest_jian_frame(&tx, &cancel, None);
        let NetEvent::Tsunami(tsu2) = rx.try_recv().expect("tsu2") else {
            panic!("expected Tsunami");
        };
        assert!(tsu2.cancelled);
    }


}
