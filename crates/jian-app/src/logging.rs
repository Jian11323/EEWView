//! 文件 + 可选控制台的 tracing 初始化（便携版无黑窗时仍可留档）。

use jian_config::LogConfig;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// 解析日志目录：绝对路径原样；相对路径相对资源根。
pub fn resolve_log_dir(resource_root: &Path, cfg: &LogConfig) -> PathBuf {
    let raw = cfg.dir.trim();
    let dir = if raw.is_empty() {
        resource_root.join("logs")
    } else {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            resource_root.join(p)
        }
    };
    dir
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
    // `RUST_LOG` 优先；否则用配置等级（全局）。
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
        if !name.starts_with("eewview.log") {
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

/// 初始化全局 subscriber；返回的 guard 须持有至进程结束（禁用文件日志时仍返回 dummy guard）。
pub fn init_logging(resource_root: &Path, cfg: &LogConfig) -> WorkerGuard {
    let log_dir = resolve_log_dir(resource_root, cfg);
    if cfg.enabled {
        if let Err(e) = fs::create_dir_all(&log_dir) {
            eprintln!("无法创建日志目录 {}: {e}", log_dir.display());
        } else {
            prune_old_logs(&log_dir, cfg.keep_days);
        }
    }

    let filter = build_filter(cfg);
    // debug 构建默认也打 stderr；release 仅当配置显式开启
    let want_console = cfg.console || cfg!(debug_assertions);

    let (nb_writer, guard) = if cfg.enabled {
        let file_appender = tracing_appender::rolling::daily(&log_dir, "eewview.log");
        tracing_appender::non_blocking(file_appender)
    } else {
        // 占位 guard：文件关闭时仍保持类型一致
        tracing_appender::non_blocking(std::io::sink())
    };

    let file_layer = cfg.enabled.then(|| {
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

    // 全关时仍挂一层 sink，保证 set_global_default 成功
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
    }));

    tracing::info!(
        dir = %log_dir.display(),
        level = %level_token(&cfg.level),
        enabled = cfg.enabled,
        console = want_console,
        keep_days = cfg.keep_days,
        "logging ready"
    );
    guard
}

/// 在资源管理器 / 文件管理器中打开日志目录。
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
