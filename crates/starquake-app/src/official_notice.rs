//! 官方通知中心：每 15 秒拉取远端生效通知，本地维护已读。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use starquake_config::AppConfig;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const POLL_INTERVAL: Duration = Duration::from_secs(15);

const FETCH_URL: &str = "https://api.sismotide.top/api/v1/notifications/updates";

const UA: &str = concat!(
    "StarQuake/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Jian11323/StarQuake)"
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteNotice {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub content_format: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub is_active: bool,
}

impl RemoteNotice {
    pub fn type_label(&self) -> &'static str {
        match self.r#type.to_ascii_uppercase().as_str() {
            "UPDATE" => "更新通知",
            "BUGFIX" => "修复通知",
            "SECURITY" => "安全通知",
            "TEST" => "测试通知",
            "NOTICE" | "INFO" => "公告通知",
            _ => "其他通知",
        }
    }

    /// 将远端 ISO/UTC 时间转为东八区展示（`YYYY-MM-DD HH:MM:SS`）。
    pub fn created_at_display(&self) -> String {
        format_notice_time_cst(&self.created_at)
    }
}

/// 解析通知时间戳并格式化为 UTC+8；无法解析时原样返回。
pub fn format_notice_time_cst(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|| {
            // 兼容无偏移 / 仅以 Z 结尾的 ISO
            let norm = s.trim_end_matches('Z').trim_end_matches('z');
            let candidates = [
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y-%m-%dT%H:%M:%S",
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%d %H:%M:%S",
            ];
            for fmt in candidates {
                if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(norm, fmt) {
                    return Some(naive.and_utc());
                }
            }
            None
        });
    let Some(utc) = parsed else {
        return s.to_string();
    };
    let cst = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    utc.with_timezone(&cst)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ReadFile {
    #[serde(default)]
    read_ids: HashSet<i64>,
}

pub struct OfficialNoticeState {
    pub items: Vec<RemoteNotice>,
    read_ids: HashSet<i64>,
    /// 待弹窗队列（按 id 去重）
    pub popup_queue: Vec<RemoteNotice>,
    last_poll: Option<Instant>,
    path: Option<PathBuf>,
}

impl OfficialNoticeState {
    pub fn load() -> Self {
        let path = AppConfig::user_app_dir().map(|d| d.join("notifications_read.json"));
        let read_ids = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<ReadFile>(&s).ok())
            .map(|f| f.read_ids)
            .unwrap_or_default();
        Self {
            items: Vec::new(),
            read_ids,
            popup_queue: Vec::new(),
            last_poll: None,
            path,
        }
    }

    pub fn is_read(&self, id: i64) -> bool {
        self.read_ids.contains(&id)
    }

    pub fn mark_read(&mut self, id: i64) {
        if self.read_ids.insert(id) {
            self.persist();
        }
        self.popup_queue.retain(|n| n.id != id);
    }

    pub fn mark_all_read(&mut self) {
        let mut changed = false;
        for n in &self.items {
            if self.read_ids.insert(n.id) {
                changed = true;
            }
        }
        self.popup_queue.clear();
        if changed {
            self.persist();
        }
    }

    fn persist(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let f = ReadFile {
            read_ids: self.read_ids.clone(),
        };
        if let Ok(s) = serde_json::to_string_pretty(&f) {
            let _ = fs::write(path, s);
        }
    }

    pub fn needs_poll(&self) -> bool {
        match self.last_poll {
            None => true,
            Some(t) => t.elapsed() >= POLL_INTERVAL,
        }
    }

    pub fn apply_fetched(&mut self, items: Vec<RemoteNotice>) {
        let cold_start = self.items.is_empty();
        let prev_ids: HashSet<i64> = self.items.iter().map(|n| n.id).collect();
        self.last_poll = Some(Instant::now());
        // 仅保留远端仍存在的已读 id，避免无限膨胀
        let alive: HashSet<i64> = items.iter().map(|n| n.id).collect();
        self.read_ids.retain(|id| alive.contains(id));

        self.items = items;
        if cold_start {
            // 冷启动：只弹最新一条，其余留在通知中心作未读
            if let Some(latest) = self.items.first().cloned() {
                if !self.read_ids.contains(&latest.id)
                    && !self.popup_queue.iter().any(|q| q.id == latest.id)
                {
                    self.popup_queue.push(latest);
                }
            }
        } else {
            for n in &self.items {
                if prev_ids.contains(&n.id) {
                    continue;
                }
                if self.read_ids.contains(&n.id) {
                    continue;
                }
                if self.popup_queue.iter().any(|q| q.id == n.id) {
                    continue;
                }
                self.popup_queue.push(n.clone());
            }
        }
        self.persist();
    }
}

pub fn fetch_notices() -> Result<Vec<RemoteNotice>> {
    let ver = env!("CARGO_PKG_VERSION");
    let url = format!("{FETCH_URL}?client_version={ver}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(UA)
        .build()
        .context("build http client")?;
    let resp = client.get(&url).send().context("get notifications")?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }
    let items: Vec<RemoteNotice> = resp.json().context("parse notifications json")?;
    let mut out: Vec<RemoteNotice> = items
        .into_iter()
        .filter(|n| n.is_active && n.id > 0 && !n.title.trim().is_empty())
        .collect();
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(out)
}
