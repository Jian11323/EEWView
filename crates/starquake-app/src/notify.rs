//! 桌面系统通知（Windows Toast）。

use tracing::{debug, warn};

/// 与安装器快捷方式 / 进程 AUMID 一致，否则 Win10/11 Toast 常静默失败。
pub const APP_USER_MODEL_ID: &str = "StarQuake.Desktop";

/// 进程启动时注册 AppUserModelID（须在首条 Toast 之前）。
pub fn register_app_id() {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = std::ffi::OsStr::new(APP_USER_MODEL_ID)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // shell32 SetCurrentProcessExplicitAppUserModelID
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
    #[cfg(windows)]
    {
        match notify_rust::Notification::new()
            .app_id(APP_USER_MODEL_ID)
            .summary(title)
            .body(body)
            .timeout(notify_rust::Timeout::Milliseconds(8_000))
            .show()
        {
            Ok(_) => {}
            Err(e) => {
                // 无自定义 AUMID 再试一次（部分环境仅接受默认）
                match notify_rust::Notification::new()
                    .summary(title)
                    .body(body)
                    .timeout(notify_rust::Timeout::Milliseconds(8_000))
                    .show()
                {
                    Ok(_) => {}
                    Err(e2) => warn!(
                        error = %e,
                        fallback = %e2,
                        "system notification failed"
                    ),
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body);
        debug!("system notification skipped (non-windows)");
    }
}
