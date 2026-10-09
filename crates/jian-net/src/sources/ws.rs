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

/// 收到文本帧时调用；返回 true 表示该帧为业务相关（可把健康态标为正常）
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
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(t)) => {
                            on_text(&tx, t.as_str());
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
                emit_health(&tx, opts.source, HealthStatus::Fluctuating);
            }
            Err(e) => {
                warn!(source = opts.source, "connect failed: {e}");
                emit_health(&tx, opts.source, HealthStatus::Abnormal);
            }
        }

        sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(opts.backoff_max_secs);
    }
}
