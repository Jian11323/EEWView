//! WebSocket 重连循环（公共）。

use futures_util::{SinkExt, StreamExt};
use jian_core::HealthStatus;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

use crate::bus::{emit_health, NetTx};

pub struct WsLoopOpts {
    pub source: &'static str,
    pub url: String,
    /// 连接成功后立刻发送的文本（鉴权 / query）
    pub on_open_text: Option<String>,
    pub backoff_max_secs: u64,
}

/// 收到文本帧时调用
pub async fn run_ws_loop<F>(tx: NetTx, opts: WsLoopOpts, mut on_text: F)
where
    F: FnMut(&NetTx, &str) + Send + 'static,
{
    let mut backoff = 1u64;
    loop {
        emit_health(&tx, opts.source, HealthStatus::Fluctuating);
        info!(source = opts.source, url = %opts.url, "connecting");

        match connect_async(&opts.url).await {
            Ok((ws, _)) => {
                emit_health(&tx, opts.source, HealthStatus::Normal);
                backoff = 1;
                let (mut write, mut read) = ws.split();
                if let Some(text) = &opts.on_open_text {
                    if let Err(e) = write.send(Message::Text(text.clone().into())).await {
                        warn!(source = opts.source, "on_open send failed: {e}");
                    }
                }
                let mut rate_limited = false;
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(t)) => {
                            let text = t.as_str();
                            if is_conn_limit_error(text) {
                                warn!(
                                    source = opts.source,
                                    "服务器连接数超限，请关闭其它占用同令牌或同 IP 的客户端后重试"
                                );
                                emit_health(&tx, opts.source, HealthStatus::Abnormal);
                                rate_limited = true;
                                break;
                            }
                            if is_error_frame(text) {
                                let msg = error_message(text).unwrap_or("error");
                                warn!(source = opts.source, message = %msg, "上游 error 帧");
                                emit_health(&tx, opts.source, HealthStatus::Abnormal);
                            }
                            on_text(&tx, text);
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = write.send(Message::Pong(p)).await;
                        }
                        Ok(Message::Close(_)) => {
                            warn!(source = opts.source, "server closed");
                            break;
                        }
                        Err(e) => {
                            warn!(source = opts.source, "ws error: {e}");
                            break;
                        }
                        _ => {}
                    }
                }
                if rate_limited {
                    backoff = backoff.max(45);
                }
                emit_health(&tx, opts.source, HealthStatus::Fluctuating);
            }
            Err(e) => {
                warn!(source = opts.source, "connect failed: {e}");
                emit_health(&tx, opts.source, HealthStatus::Abnormal);
            }
        }

        sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(opts.backoff_max_secs.max(60));
    }
}

fn is_error_frame(text: &str) -> bool {
    text.contains("\"type\":\"error\"") || text.contains("\"type\": \"error\"")
}

fn is_conn_limit_error(text: &str) -> bool {
    if !is_error_frame(text) {
        return false;
    }
    text.contains("连接限制")
        || text.contains("conn_limit")
        || text.contains("超过服务器连接")
        || text.contains("max_connections")
}

fn error_message(text: &str) -> Option<&str> {
    let key = "\"message\"";
    let i = text.find(key)?;
    let rest = &text[i + key.len()..];
    let start = rest.find('"')? + 1;
    let rest = &rest[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}
