//! 文件 + 可选控制台的 tracing 初始化。
//!
//! Windows 默认写入 `%APPDATA%\eew view\log.txt`（安装到 Program Files 时资源根不可写，
//! 若仍向安装目录建 logs/ 会令 tracing-appender panic，表现为双击无反应）。

use jian_config::{AppConfig, LogConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// 用户目录下的单一日志文件：`…/eew view/log.txt`。
fn user_log_file() -> Option<PathBuf> {
    AppConfig::user_app_dir().map(|d| d.join("log.txt"))
}

fn preferred_log_file(resource_root: &Path, cfg: &LogConfig) -> PathBuf {
    let raw = cfg.dir.trim();
    if !raw.is_empty() {
        let p = Path::new(raw);
        if p.is_absolute() {
            // 绝对路径：若像目录则拼 log.txt，否则视为文件
            if p.extension().is_none() || p.is_dir() {
                return p.join("log.txt");
            }
            return p.to_path_buf();
        }
    }
    if let Some(f) = user_log_file() {
        return f;
    }
    // 无用户目录时退回资源根（便携/特殊环境）
    if raw.is_empty() {
        resource_root.join("logs").join("log.txt")
    } else {
        resource_root.join(raw).join("log.txt")
    }
}

fn ensure_parent_writable(file: &Path) -> bool {
    let Some(dir) = file.parent() else {
        return false;
    };
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".eewview_write_probe");
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// 日志所在目录（设置页「打开日志目录」用）。
pub fn resolve_log_dir(resource_root: &Path, cfg: &LogConfig) -> PathBuf {
    let file = preferred_log_file(resource_root, cfg);
    file.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| resource_root.join("logs"))
}

fn pick_writable_log_file(resource_root: &Path, cfg: &LogConfig) -> Option<PathBuf> {
    let preferred = preferred_log_file(resource_root, cfg);
    if ensure_parent_writable(&preferred) {
        return Some(preferred);
    }
    eprintln!("无法写入日志 {}，尝试备用路径…", preferred.display());
    // 备用：资源根 logs/（便携包可写时）
    let fallback = resource_root.join("logs").join("log.txt");
    if ensure_parent_writable(&fallback) {
        eprintln!("日志改写到 {}", fallback.display());
        return Some(fallback);
    }
    None
}

fn level_token(level: &str) -> &'static str {
    match level.trim().to_ascii_lowercase().as_str() {
        "error" => "error",
        "warn" => "warn",
        "debug" => "debug",
        "trace" => "trace",
        _ => "info",
    }
}

fn build_filter(cfg: &LogConfig) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level_token(&cfg.level)))
}

fn prune_old_logs(dir: &Path, keep_days: u64) {
    if keep_days == 0 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let max_age = Duration::from_secs(keep_days.saturating_mul(24 * 3600));
    let now = SystemTime::now();
    for ent in rd.flatten() {
        let path = ent.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // 清理历史按日滚动文件；保留现行 log.txt
        if !(name.starts_with("eewview.log") || (name.starts_with("log.") && name != "log.txt")) {
            continue;
        }
        let Ok(meta) = ent.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now.duration_since(modified).unwrap_or_default() > max_age {
            let _ = fs::remove_file(&path);
        }
    }
}

/// 初始化全局 subscriber；不可写时关闭文件层，切勿让 appender panic。
pub fn init_logging(resource_root: &Path, cfg: &LogConfig) -> WorkerGuard {
    let log_file = if cfg.enabled {
        pick_writable_log_file(resource_root, cfg)
    } else {
        None
    };
    if let Some(ref file) = log_file {
        if let Some(dir) = file.parent() {
            prune_old_logs(dir, cfg.keep_days);
        }
    }

    let filter = build_filter(cfg);
    let want_console = cfg.console || cfg!(debug_assertions);
    let file_enabled = cfg.enabled && log_file.is_some();

    let (nb_writer, guard) = if let Some(ref file) = log_file {
        let dir = file.parent().unwrap_or_else(|| Path::new("."));
        let name = file
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("log.txt");
        // 单一文件、不按日滚动（安装约定：AppData\\eew view\\log.txt）
        let file_appender = tracing_appender::rolling::never(dir, name);
        tracing_appender::non_blocking(file_appender)
    } else {
        tracing_appender::non_blocking(std::io::sink())
    };

    let file_layer = file_enabled.then(|| {
        fmt::layer()
            .with_writer(nb_writer)
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
    });

    let console_layer = want_console.then(|| {
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(true)
            .with_target(true)
    });

    let sink_layer = (file_layer.is_none() && console_layer.is_none()).then(|| {
        fmt::layer()
            .with_writer(|| std::io::sink())
            .with_ansi(false)
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .with(sink_layer)
        .init();

    std::panic::set_hook(Box::new(|info| {
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "(unknown)".into());
        let payload = if let Some(s) = info.payload().downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Box<Any>".into()
        };
        tracing::error!(%loc, %payload, "panic");
        eprintln!("panic at {loc}: {payload}");
        show_fatal_dialog(
            "EEW View",
            &format!("程序异常退出：\n{payload}\n\n位置：{loc}"),
        );
    }));

    let log_display = log_file
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(disabled)".into());
    tracing::info!(
        file = %log_display,
        level = %level_token(&cfg.level),
        enabled = file_enabled,
        console = want_console,
        keep_days = cfg.keep_days,
        "logging ready"
    );
    guard
}

pub fn open_log_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(dir).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(dir).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}

pub fn show_fatal_dialog(title: &str, message: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        fn wide(s: &str) -> Vec<u16> {
            std::ffi::OsStr::new(s)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                hwnd: *mut core::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                flags: u32,
            ) -> i32;
        }
        const MB_ICONERROR: u32 = 0x0000_0010;
        const MB_OK: u32 = 0x0000_0000;
        let text = wide(message);
        let caption = wide(title);
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                caption.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (title, message);
        eprintln!("{title}: {message}");
    }
}
