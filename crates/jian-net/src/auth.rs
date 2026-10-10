//! Jian Project 鉴权：`lk_` → `rt_` → `at_`（auth.sismotide.top）。

use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

const REFRESH_URL: &str = "https://auth.sismotide.top/api/refresh";
const ACCESS_URL: &str = "https://auth.sismotide.top/api/access";
const UA: &str = "EEWView/0.1";

#[derive(Debug, Clone)]
pub struct TokenOk {
    pub token: String,
    /// 过期时刻（unix 秒）；未知则为 0
    pub expire_at: f64,
    pub message: String,
}

#[derive(Deserialize)]
struct AuthBody {
    ok: Option<bool>,
    token: Option<String>,
    error: Option<String>,
    message: Option<String>,
    code: Option<serde_json::Value>,
    expires_after_sec: Option<i64>,
    expires_after_min: Option<i64>,
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn classify(value: &str) -> &str {
    let v = value.trim();
    if v.starts_with("lk_") {
        "login"
    } else if v.starts_with("rt_") {
        "refresh"
    } else if v.starts_with("at_") {
        "access"
    } else {
        "unknown"
    }
}

fn parse_body(body: AuthBody) -> Result<TokenOk, String> {
    if body.ok == Some(false) {
        let mut err = body
            .error
            .or(body.message)
            .unwrap_or_else(|| "鉴权失败".into());
        if let Some(code) = body.code {
            err = format!("{err} (code={code})");
        }
        return Err(err);
    }
    let token = body.token.unwrap_or_default().trim().to_string();
    if token.is_empty() {
        return Err(body
            .error
            .or(body.message)
            .unwrap_or_else(|| "未返回令牌".into()));
    }
    let expire_at = if let Some(sec) = body.expires_after_sec {
        now_unix() + sec.max(0) as f64
    } else if let Some(min) = body.expires_after_min {
        now_unix() + (min.max(0) as f64) * 60.0
    } else {
        0.0
    };
    Ok(TokenOk {
        token,
        expire_at,
        message: "OK".into(),
    })
}

fn post_bearer(url: &str, token: &str) -> Result<TokenOk, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .header("Authorization", format!("Bearer {}", token.trim()))
        .header("Accept", "application/json")
        .send()
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let body: AuthBody = resp.json().map_err(|e| {
        if !status.is_success() {
            format!("HTTP {status}")
        } else {
            e.to_string()
        }
    })?;
    parse_body(body)
}

/// 用邮件登录密钥 `lk_` 换取长期 Token `rt_`（约 180 天）。
pub fn exchange_login_key(login_key: &str) -> Result<TokenOk, String> {
    let key = login_key.trim();
    if key.is_empty() {
        return Err("请先填写 Jian 登录密钥（邮件内 lk_…）".into());
    }
    if classify(key) != "login" {
        return Err("请填写邮件内 lk_ 开头的登录密钥（约 5 分钟有效、用后作废）".into());
    }
    let mut ok = post_bearer(REFRESH_URL, key)?;
    if !ok.token.starts_with("rt_") {
        return Err(format!("期望 rt_ 长期 Token，收到：{}", &ok.token[..ok.token.len().min(8)]));
    }
    if ok.expire_at <= 0.0 {
        // 文档默认 180 天
        ok.expire_at = now_unix() + 259200.0 * 60.0;
    }
    ok.message = "鉴权成功，已保存长期 Token。".into();
    Ok(ok)
}

/// 用长期 Token `rt_` 换取访问令牌 `at_`（约 1 小时）。
pub fn exchange_access_token(refresh_token: &str) -> Result<TokenOk, String> {
    let token = refresh_token.trim();
    if token.is_empty() {
        return Err("长期 Token 为空".into());
    }
    if classify(token) != "refresh" {
        return Err("长期 Token 须为 rt_ 开头".into());
    }
    let ok = post_bearer(ACCESS_URL, token)?;
    if !ok.token.starts_with("at_") {
        return Err(format!("期望 at_ 访问令牌，收到：{}", &ok.token[..ok.token.len().min(8)]));
    }
    Ok(ok)
}

/// 验证已保存的 `rt_`（换一张 at_ 即视为有效）。
pub fn verify_refresh_token(refresh_token: &str) -> Result<TokenOk, String> {
    let _ = exchange_access_token(refresh_token)?;
    Ok(TokenOk {
        token: refresh_token.trim().to_string(),
        expire_at: 0.0,
        message: "长期 Token 有效。".into(),
    })
}

/// 格式化 rt_ 过期日期（本地）。
pub fn format_expire_date(expire_at: f64) -> Option<String> {
    if expire_at <= 0.0 {
        return None;
    }
    let secs = expire_at as i64;
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| {
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string()
    })
}
