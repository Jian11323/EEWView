//! 桌面系统通知（Windows Toast）。

use tracing::debug;

/// 弹出系统通知。失败时仅记日志，不阻断主循环。
pub fn show(title: &str, body: &str) {
    #[cfg(windows)]
    {
        match notify_rust::Notification::new()
            .app_id("StarQuake.Desktop")
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8_000))
            .show()
        {
            Ok(_) => {}
            Err(e) => debug!(error = %e, "system notification failed"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body);
        debug!("system notification skipped (non-windows)");
    }
}
