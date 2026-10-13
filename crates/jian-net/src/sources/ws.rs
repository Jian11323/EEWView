//! WebSocket 重连循环（公共）。
//!
//! 对齐 EEWCN 测站链路：应用层看门狗、指数退避 + 抖动，僵死连接及时重建。

use futures_util::{SinkExt, StreamExt};
use jian_core::HealthStatus;
use std::time::{Duration, Instant};
use tokio::time::{interval, sleep, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

use crate::bus::{emit_health, NetTx};

pub struct WsLoopOpts {
    pub source: &'static str,
    pub url: String,
    /// 连接成功后立刻发送的文本（鉴权 / query / list）
    pub on_open_texts: Vec<String>,
    /// 连接存活期间周期性重发（如 Jian cenclist）；空则不发
    pub periodic_texts: Vec<String>,
    /// 周期秒数；0 或 periodic_texts 空则禁用
    pub periodic_secs: u64,
    pub backoff_max_secs: u64,
    /// 无任何下行帧（含 ping/pong/heartbeat）超过该秒数则主动断开重连；0=关闭。
    /// kmoni/kma 建议 15；s-net 约每分钟一帧，建议 120。
    pub stale_secs: u64,
}

impl WsLoopOpts {
    pub fn with_stale(mut self, secs: u64) -> Self {
        self.stale_secs = secs;
        self
    }
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
                for text in &opts.on_open_texts {
                    if let Err(e) = write.send(Message::Text(text.clone().into())).await {
                        warn!(source = opts.source, "on_open send failed: {e}");
                    }
                }
                let mut rate_limited = false;
                let use_periodic =
                    opts.periodic_secs > 0 && !opts.periodic_texts.is_empty();
                let mut tick = if use_periodic {
                    let mut t = interval(Duration::from_secs(opts.periodic_secs));
                    t.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    // 跳过立刻触发的首 tick（on_open 已发过）
                    t.tick().await;
                    Some(t)
                } else {
                    None
                };
                let stale = if opts.stale_secs > 0 {
                    Duration::from_secs(opts.stale_secs)
                } else {
                    Duration::from_secs(u64::MAX / 4)
                };
                let mut last_msg = Instant::now();
                // 首包宽限：台账大帧可能较慢（对齐 EEWCN awaitFirstMessage）
                let first_grace = Duration::from_secs(opts.stale_secs.max(20).min(90));
                let opened_at = Instant::now();
                let mut got_first = false;
                loop {
                    let watchdog = sleep_until_deadline(last_msg, if got_first { stale } else { first_grace });
                    tokio::select! {
                        msg = read.next() => {
                            match msg {
                                Some(Ok(Message::Text(t))) => {
                                    last_msg = Instant::now();
                                    got_first = true;
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
                                Some(Ok(Message::Ping(p))) => {
                                    last_msg = Instant::now();
                                    got_first = true;
                                    let _ = write.send(Message::Pong(p)).await;
                                }
                                Some(Ok(Message::Pong(_))) => {
                                    last_msg = Instant::now();
                                    got_first = true;
                                }
                                Some(Ok(Message::Close(_))) => {
                                    warn!(source = opts.source, "server closed");
                                    break;
                                }
                                Some(Err(e)) => {
                                    warn!(source = opts.source, "ws error: {e}");
                                    break;
                                }
                                None => break,
                                _ => {}
                            }
                        }
                        _ = async {
                            if let Some(t) = tick.as_mut() {
                                t.tick().await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                        } => {
                            for text in &opts.periodic_texts {
                                if let Err(e) = write.send(Message::Text(text.clone().into())).await {
                                    warn!(source = opts.source, "periodic send failed: {e}");
                                    break;
                                }
                            }
                        }
                        _ = watchdog, if opts.stale_secs > 0 => {
                            let age = last_msg.elapsed().as_secs();
                            let open_age = opened_at.elapsed().as_secs();
                            warn!(
                                source = opts.source,
                                age_s = age,
                                open_s = open_age,
                                "watchdog stale → reconnect"
                            );
                            emit_health(&tx, opts.source, HealthStatus::Fluctuating);
                            break;
                        }
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

        let jitter_ms = (backoff.saturating_mul(137) % 800) as u64;
        sleep(Duration::from_secs(backoff) + Duration::from_millis(jitter_ms)).await;
        backoff = (backoff * 2).min(opts.backoff_max_secs.max(60));
    }
}

async fn sleep_until_deadline(from: Instant, window: Duration) {
    let deadline = from + window;
    let now = Instant::now();
    if deadline > now {
        sleep(deadline - now).await;
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
