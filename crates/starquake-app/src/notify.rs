//! 桌面系统通知（对齐滚动字幕 `utils/desktop_notify.py`）。
//!
//! 路径优先级：
//! 1. 托盘 balloon（`QSystemTrayIcon.showMessage` / `Shell_NotifyIcon` NIF_INFO）
//! 2. notify-rust WinRT Toast（回退）

use tracing::{debug, info, warn};

/// 与滚动字幕同样在进程启动时注册显式 AUMID，提升 Toast 关联稳定性。
pub const APP_USER_MODEL_ID: &str = "StarQuake.Desktop";

/// 须在首条通知前调用（对齐滚动字幕 `ensure_windows_app_id`）。
pub fn register_app_id() {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(APP_USER_MODEL_ID)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        #[link(name = "shell32")]
        extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(app_id: *const u16) -> i32;
        }
        let hr = unsafe { SetCurrentProcessExplicitAppUserModelID(wide.as_ptr()) };
        if hr < 0 {
            warn!(hr, "SetCurrentProcessExplicitAppUserModelID failed");
        } else {
            debug!("registered AppUserModelID {}", APP_USER_MODEL_ID);
        }
    }
}

/// 弹出系统通知。失败时记 warn，不阻断主循环。
pub fn show(title: &str, body: &str) {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() && body.is_empty() {
        return;
    }
    let title = if title.is_empty() { "StarQuake" } else { title };
    let title = &title[..title.len().min(120)];
    let body = &body[..body.len().min(500)];

    #[cfg(windows)]
    {
        // 1) 托盘 balloon（Win10/11 通用，对齐滚动字幕默认路径）
        if crate::tray::show_balloon(title, body) {
            info!(title, "system notification via tray balloon");
            return;
        }
        // 2) WinRT / notify-rust 回退
        match notify_rust::Notification::new()
            .app_id(APP_USER_MODEL_ID)
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8_000))
            .show()
        {
            Ok(_) => {
                info!(title, "system notification via winrt");
                return;
            }
            Err(e) => {
                debug!(error = %e, "winrt toast with AUMID failed, retry default");
            }
        }
        match notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8_000))
            .show()
        {
            Ok(_) => info!(title, "system notification via winrt default"),
            Err(e) => warn!(error = %e, "system notification failed"),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body);
        debug!("system notification skipped (non-windows)");
    }
}

/// 设置页试听。
pub fn show_test() {
    show(
        "StarQuake",
        "\u{7cfb}\u{7edf}\u{901a}\u{77e5}\u{6d4b}\u{8bd5}\u{ff1a}\u{82e5}\u{60a8}\u{770b}\u{5230}\u{672c}\u{6761}\u{ff0c}\u{684c}\u{9762}\u{901a}\u{77e5}\u{5df2}\u{53ef}\u{7528}\u{3002}",
    );
}
