//! 机构展示标签与过滤族。

/// 预警机构开关（Jian `/all` 内各 EEW 源）。
pub const EEW_FILTER_FAMILIES: [AgencyFamily; 6] = [
    AgencyFamily::Jma,
    AgencyFamily::Cn,
    AgencyFamily::Cwa,
    AgencyFamily::Kma,
    AgencyFamily::EarlyEst,
    AgencyFamily::Sa,
];

/// 速报震级过滤（对齐 RhythmQuake「信息事件震级过滤」）。
pub const RECORD_FILTER_FAMILIES: [AgencyFamily; 10] = [
    AgencyFamily::Cn,
    AgencyFamily::Usgs,
    AgencyFamily::Cwa,
    AgencyFamily::Jma,
    AgencyFamily::Hko,
    AgencyFamily::Emsc,
    AgencyFamily::Ningxia,
    AgencyFamily::Yunnan,
    AgencyFamily::Beijing,
    AgencyFamily::Other,
];

/// 地名后追加的机构括号标签，如 `[JMA]`、`[CN]`、`[四川地震局]`。
pub fn agency_bracket(agency: &str) -> String {
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
        AgencyFamily::Jma => "JMA".into(),
        AgencyFamily::Cn => "CN".into(),
        AgencyFamily::Usgs => "USGS".into(),
        AgencyFamily::Cwa => "CWA".into(),
        AgencyFamily::Kma => "KMA".into(),
        AgencyFamily::EarlyEst => "Early-est".into(),
        AgencyFamily::Ningxia => "宁夏地震局".into(),
        AgencyFamily::Yunnan => "云南地震局".into(),
        AgencyFamily::Beijing => "北京地震局".into(),
        AgencyFamily::Hko => "HKO".into(),
        AgencyFamily::Emsc => "EMSC".into(),
        AgencyFamily::Sa => "SA".into(),
        AgencyFamily::Other => "其他".into(),
    }
}

/// 从 `cea-pr` / `cea-pr:四川` 解析省份；非 cea-pr 返回 `None`。
fn cea_pr_province(agency: &str) -> Option<&str> {
    let a = agency.trim();
    let lower = a.to_ascii_lowercase().replace('_', "-");
    if lower == "cea-pr" {
        return Some("");
    }
    if let Some(rest) = lower.strip_prefix("cea-pr:") {
        let start = a.len().saturating_sub(rest.len());
        return Some(a[start..].trim());
    }
    if let Some(rest) = lower.strip_prefix("cea-pr/") {
        let start = a.len().saturating_sub(rest.len());
        return Some(a[start..].trim());
    }
    None
}

/// 组装 cea-pr 的 agency 存储键（含省份时为 `cea-pr:四川`）。
pub fn cea_pr_agency_id(province: Option<&str>) -> String {
    match province.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("cea-pr:{p}"),
        None => "cea-pr".into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgencyFamily {
    Jma,
    Cn,
    Usgs,
    Cwa,
    Kma,
    EarlyEst,
    Ningxia,
    Yunnan,
    Beijing,
    Hko,
    Emsc,
    Sa,
    Other,
}

impl AgencyFamily {
    pub fn as_filter_key(self) -> &'static str {
        match self {
            Self::Jma => "jma",
            Self::Cn => "cenc",
            Self::Usgs => "usgs",
            Self::Cwa => "cwa",
            Self::Kma => "kma",
            Self::EarlyEst => "early_est",
            Self::Ningxia => "ningxia",
            Self::Yunnan => "yunnan",
            Self::Beijing => "beijing",
            Self::Hko => "hko",
            Self::Emsc => "emsc",
            Self::Sa => "sa",
            Self::Other => "other",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Jma => "气象厅 / P2PQuake",
            Self::Cn => "中国地震台网 (CENC/CEA)",
            Self::Usgs => "USGS (美国)",
            Self::Cwa => "中央气象署 (CWA)",
            Self::Kma => "韩国气象厅 (KMA)",
            Self::EarlyEst => "Early-est",
            Self::Ningxia => "宁夏地震局",
            Self::Yunnan => "云南地震局",
            Self::Beijing => "北京地震局",
            Self::Hko => "香港天文台 (HKO)",
            Self::Emsc => "欧洲地中海地震中心",
            Self::Sa => "四川地震局 (SA)",
            Self::Other => "未适配机构",
        }
    }
}

pub fn agency_family(agency: &str) -> AgencyFamily {
    let a = agency.trim().to_ascii_lowercase().replace('_', "-");
    if a.contains("early-est") || a.contains("earlyest") {
        return AgencyFamily::EarlyEst;
    }
    if a.contains("usgs") {
        return AgencyFamily::Usgs;
    }
    if a.contains("cwa") {
        return AgencyFamily::Cwa;
    }
    if a.contains("kma") {
        return AgencyFamily::Kma;
    }
    if a.contains("ningxia") {
        return AgencyFamily::Ningxia;
    }
    if a.contains("yunnan") {
        return AgencyFamily::Yunnan;
    }
    if a.contains("beijing") {
        return AgencyFamily::Beijing;
    }
    if a.contains("hko") {
        return AgencyFamily::Hko;
    }
    if a.contains("emsc") || a.contains("euro") {
        return AgencyFamily::Emsc;
    }
    if a == "sa" || a.starts_with("sa-") || a.ends_with("-sa") {
        return AgencyFamily::Sa;
    }
    // cea / cea-pr / cenc → 中国系（展示标签在 agency_bracket 区分）
    if a.contains("cea") || a.contains("cenc") {
        return AgencyFamily::Cn;
    }
    if a.contains("jma") || a.contains("p2p") || a.contains("wolfx-jma") {
        return AgencyFamily::Jma;
    }
    if a.contains("wolfx-cenc") || (a.contains("wolfx") && a.contains("cenc")) {
        return AgencyFamily::Cn;
    }
    AgencyFamily::Other
}

/// `马鲁古海北部 [JMA]` / `红原县 [四川地震局]`
pub fn place_with_agency(place: &str, agency: &str) -> String {
    let tag = agency_bracket(agency);
    let bracketed = format!("[{tag}]");
    if place.contains(&bracketed) {
        place.to_string()
    } else {
        format!("{place} {bracketed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cea_maps_to_cn() {
        assert_eq!(agency_bracket("cea"), "CN");
        assert_eq!(agency_family("cea"), AgencyFamily::Cn);
        assert_eq!(place_with_agency("缅甸", "cea"), "缅甸 [CN]");
    }

    #[test]
    fn cea_pr_maps_to_province_bureau() {
        assert_eq!(agency_bracket("cea-pr:四川"), "四川地震局");
        assert_eq!(agency_bracket("cea-pr"), "地方地震局");
        assert_eq!(agency_family("cea-pr:云南"), AgencyFamily::Cn);
        assert_eq!(
            place_with_agency("阿坝州红原县", "cea-pr:四川"),
            "阿坝州红原县 [四川地震局]"
        );
        assert_eq!(cea_pr_agency_id(Some("四川")), "cea-pr:四川");
    }
}
