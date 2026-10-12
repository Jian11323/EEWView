//! Windows TTS 朗读文案：机构读法 + 预警/速报脚本。

use crate::agency::{agency_family, cea_pr_province, AgencyFamily};
use crate::{EewReport, EqRecord, IntensityKind};

/// TTS 专用机构读法（与 UI 括号标签分离）。
pub fn agency_tts_name(agency: &str, is_eew: bool, is_warn: bool) -> String {
    if let Some(prov) = cea_pr_province(agency) {
        if prov.is_empty() {
            return "地方地震局".into();
        }
        if prov.ends_with("地震局") {
            return prov.to_string();
        }
        return format!("{prov}地震局");
    }
    match agency_family(agency) {
        AgencyFamily::Cea => "中国预警网".into(),
        AgencyFamily::Jma if is_eew => {
            if is_warn {
                "日本气象厅警报".into()
            } else {
                "日本气象厅预报".into()
            }
        }
        AgencyFamily::Jma => "日本气象厅".into(),
        AgencyFamily::Sa => "美国ShakeAlert预警".into(),
        AgencyFamily::Kma => "韩国气象厅".into(),
        AgencyFamily::Gq => "GlobalQuake预警".into(),
        AgencyFamily::EarlyEst => "意大利Early-est".into(),
        AgencyFamily::Cwa => "台湾中央气象署".into(),
        AgencyFamily::Cenc => "中国地震台网中心".into(),
        AgencyFamily::Ningxia => "宁夏地震局".into(),
        AgencyFamily::Beijing => "北京地震局".into(),
        AgencyFamily::Yunnan => "云南地震局".into(),
        AgencyFamily::Shanxi => "山西地震局".into(),
        AgencyFamily::Hko => "香港天文台".into(),
        AgencyFamily::Tmd => "泰国气象局".into(),
        AgencyFamily::Bmkg => "印尼气象气候与地球物理局".into(),
        AgencyFamily::Geonet => "新西兰地质灾害监测网".into(),
        AgencyFamily::Usgs => "美国地质调查局".into(),
        AgencyFamily::Emsc => "欧洲地中海地震中心".into(),
        AgencyFamily::Gfz => "德国地学研究中心".into(),
        AgencyFamily::Bcsf => "法国国家地震局".into(),
        AgencyFamily::Ingv => "意大利国家地球物理与火山研究所".into(),
        AgencyFamily::Usp => "巴西圣保罗大学".into(),
        AgencyFamily::Nrcan => "加拿大国家地震研究中心".into(),
        AgencyFamily::Afad => "土耳其地震研究中心".into(),
        AgencyFamily::Ipma => "葡萄牙气象研究所".into(),
        AgencyFamily::Noa => "希腊国家天文台".into(),
        AgencyFamily::Sed => "瑞士地震服务中心".into(),
        AgencyFamily::Scsn => "南加州地震数据中心".into(),
        AgencyFamily::Ipgp => "巴黎地球物理研究所".into(),
        AgencyFamily::Infp => "罗马尼亚国家地球物理研究所".into(),
        AgencyFamily::Isc => "国际地震中心".into(),
        AgencyFamily::Knmi => "荷兰皇家气象研究所".into(),
        AgencyFamily::Ncedc => "北加州地震数据中心".into(),
        AgencyFamily::Lmu => "慕尼黑大学地震服务中心".into(),
        AgencyFamily::Koeri => "土耳其坎迪利天文台".into(),
        AgencyFamily::Gsras => "俄罗斯科学院地球物理局".into(),
        AgencyFamily::Csn => "智利国家地震中心".into(),
        AgencyFamily::Phivolcs => "菲律宾火山地震研究所".into(),
        AgencyFamily::Ssn => "墨西哥地震服务中心".into(),
        AgencyFamily::Ga => "澳大利亚地震局".into(),
        AgencyFamily::Igepn => "厄瓜多尔国家地球物理研究所".into(),
        AgencyFamily::Peru => "秘鲁地震物理研究所".into(),
        AgencyFamily::Mmd => "马来西亚气象局".into(),
        AgencyFamily::Other => "地震情报".into(),
    }
}

fn sanitize_place(place: &str) -> String {
    let t = place
        .replace(['【', '】', '[', ']', '（', '）', '(', ')', '|', '｜'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if t.is_empty() {
        "未知地点".into()
    } else {
        t
    }
}

fn format_origin_for_tts(origin_ms: i64) -> String {
    if origin_ms <= 0 {
        return String::new();
    }
    // 按东八区读出，便于国内听感
    let sec = origin_ms / 1000 + 8 * 3600;
    let days = sec.div_euclid(86400);
    let tod = sec.rem_euclid(86400) as i64;
    // 1970-01-01 起算的简易公历（足够 TTS；极端历史可不精确）
    let (y, m, d) = civil_from_days(days);
    let hh = tod / 3600;
    let mm = (tod % 3600) / 60;
    let ss = tod % 60;
    format!("{y}年{m}月{d}日{hh}时{mm}分{ss}秒")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Howard Hinnant civil_from_days
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as i64, d as i64)
}

fn intensity_phrase(kind: IntensityKind, text: &str, eew: bool) -> String {
    let t = text.trim();
    if t.is_empty() || t == "—" || t == "-" || t == "?" {
        return String::new();
    }
    match kind {
        IntensityKind::CnIntensity => {
            if eew {
                format!("预估最大烈度{t}")
            } else {
                format!("烈度{t}")
            }
        }
        IntensityKind::JmaShindo => {
            if eew {
                format!("推定最大震度{t}")
            } else {
                format!("最大震度{t}")
            }
        }
    }
}

/// 预警 TTS：`source[预警]第N报，place发生M级地震。预估最大烈度/推定最大震度…`
pub fn build_eew_tts_script(ev: &EewReport) -> String {
    let src = agency_tts_name(&ev.agency.0, true, ev.is_warn);
    let is_jma_eew = agency_family(&ev.agency.0) == AgencyFamily::Jma;
    let org = if is_jma_eew {
        src
    } else if src.contains("预警") {
        src
    } else {
        format!("{src}预警")
    };
    let updates = if ev.is_final && is_jma_eew {
        "最终报".to_string()
    } else if ev.serial > 0 {
        format!("第{}报", ev.serial)
    } else {
        String::new()
    };
    let place = sanitize_place(&ev.place);
    let body = if ev.magnitude > 0.0 {
        format!("{place}发生{:.1}级地震", ev.magnitude)
    } else {
        format!("{place}发生地震")
    };
    let inten = intensity_phrase(ev.intensity_kind, &ev.max_intensity_text, true);
    let mut out = if updates.is_empty() {
        format!("{org}，{body}")
    } else {
        format!("{org}{updates}，{body}")
    };
    if !inten.is_empty() {
        out.push('。');
        out.push_str(&inten);
    }
    out
}

/// 速报 TTS：`source地震速报[自动/正式测定]，时间 place发生M级…，震源深度…，震度/烈度…`
pub fn build_record_tts_script(rec: &EqRecord) -> String {
    let src = agency_tts_name(&rec.agency.0, false, false);
    let det = if agency_family(&rec.agency.0) == AgencyFamily::Cenc {
        if rec.is_auto {
            "[自动测定]"
        } else {
            "[正式测定]"
        }
    } else {
        ""
    };
    let org = if src.contains("地震速报") || src.contains("地震信息") {
        format!("{src}{det}")
    } else {
        format!("{src}地震速报{det}")
    };
    let time = format_origin_for_tts(rec.origin_ms);
    let place = sanitize_place(&rec.place);
    let mut body = if rec.magnitude > 0.0 {
        format!("{place}发生{:.1}级地震", rec.magnitude)
    } else {
        format!("{place}发生地震")
    };
    if rec.depth_km >= 0.0 {
        body.push_str(&format!("，震源深度{:.0}千米", rec.depth_km));
    }
    let inten = intensity_phrase(rec.intensity_kind, &rec.intensity_text, false);
    if !inten.is_empty() {
        body.push('，');
        body.push_str(&inten);
    }
    if time.is_empty() {
        format!("{org}，{body}")
    } else {
        format!("{org}，{time} {body}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgencyId;

    #[test]
    fn jma_eew_no_extra_yujing() {
        let ev = EewReport {
            agency: AgencyId("jma-eew".into()),
            event_id: "1".into(),
            serial: 3,
            place: "茨城县北部".into(),
            magnitude: 5.1,
            depth_km: 10.0,
            latitude: 36.0,
            longitude: 140.0,
            origin_ms: 0,
            max_intensity_text: "4".into(),
            intensity_kind: IntensityKind::JmaShindo,
            intensity_level: 4,
            is_final: false,
            is_cancel: false,
            is_assumption: false,
            is_warn: false,
            final_at_ms: 0,
        };
        let s = build_eew_tts_script(&ev);
        assert!(s.contains("日本气象厅预报"));
        assert!(!s.contains("预报预警"));
        assert!(s.contains("第3报"));
        assert!(s.contains("推定最大震度4"));
    }

    #[test]
    fn sa_tts_name() {
        assert_eq!(
            agency_tts_name("sa", true, false),
            "美国ShakeAlert预警"
        );
    }
}
