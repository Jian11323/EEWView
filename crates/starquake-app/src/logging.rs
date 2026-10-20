//! 文件 + 可选控制台的 tracing 初始化。
//!
//! Windows 默认写入 `%APPDATA%\StarQuake\log.txt`（安装到 Program Files 时资源根不可写，
//! 若仍向安装目录建 logs/ 会令 tracing-appender panic，表现为双击无反应）。
//!
//! 格式约定：本地时间、UTF-8 BOM、CRLF（记事本可读）；写入前统一脱敏令牌。

use starquake_config::{AppConfig, LogConfig};
use starquake_util::redact_secrets;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::Writer as FmtWriter;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// 运行时可切换：tracing 控制台层始终挂载，由此门闩决定是否真正写 stderr。
static CONSOLE_ENABLED: AtomicBool = AtomicBool::new(false);

/// 热切换 stderr 输出；Windows 上不同步分配控制台窗口。
pub fn set_console_output(want: bool) {
    CONSOLE_ENABLED.store(want, Ordering::SeqCst);
    sync_windows_console(want);
}

/// 用户目录下的单一日志文件：`…/StarQuake/log.txt`。
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
    let probe = dir.join(".starquake_write_probe");
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
        if !(name.starts_with("starquake.log")
            || name.starts_with("eewview.log")
            || (name.starts_with("log.") && name != "log.txt"))
        {
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

/// 启动时：脱敏历史令牌 + 补 UTF-8 BOM + Windows 换 CRLF，避免记事本乱码/粘行。
fn scrub_existing_log_file(path: &Path) {
    let Ok(raw) = fs::read(path) else {
        return;
    };
    if raw.is_empty() {
        return;
    }
    let has_bom = raw.starts_with(&[0xEF, 0xBB, 0xBF]);
    let body = if has_bom { &raw[3..] } else { raw.as_slice() };
    let text = match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        Err(_) => return, // 非 UTF-8 不改写，避免二次损坏
    };
    let scrubbed = redact_secrets(&text);
    let needs_scrub = scrubbed != text;
    let needs_bom = !has_bom;
    let needs_crlf = cfg!(windows) && scrubbed.contains('\n') && !scrubbed.contains("\r\n");
    if !needs_scrub && !needs_bom && !needs_crlf {
        return;
    }
    let line_body = if needs_crlf {
        to_crlf(&scrubbed)
    } else {
        scrubbed
    };
    let mut out = Vec::with_capacity(line_body.len() + 3);
    out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    out.extend_from_slice(line_body.as_bytes());
    let _ = fs::write(path, out);
}

fn to_crlf(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 32);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                out.push('\r');
                if chars.peek() == Some(&'\n') {
                    out.push(chars.next().unwrap());
                }
            }
            '\n' => {
                out.push('\r');
                out.push('\n');
            }
            _ => out.push(c),
        }
    }
    out
}

/// 本地时间戳（避免 UTC `Z` 难读）。
struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut FmtWriter<'_>) -> std::fmt::Result {
        let now = chrono::Local::now();
        write!(w, "{}", now.format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

struct SafeFileInner {
    file: File,
    /// 按行缓冲后再脱敏，避免 `key=at_…` 被拆到两次 write 而漏出。
    pending: Vec<u8>,
}

/// 文件写入：按行脱敏 → CRLF → UTF-8。
#[derive(Clone)]
struct SafeFileWriter {
    inner: Arc<Mutex<SafeFileInner>>,
}

impl SafeFileWriter {
    fn open(path: &Path) -> io::Result<Self> {
        scrub_existing_log_file(path);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len == 0 {
            file.write_all(&[0xEF, 0xBB, 0xBF])?;
            file.flush()?;
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(SafeFileInner {
                file,
                pending: Vec::with_capacity(512),
            })),
        })
    }
}

fn flush_complete_lines(inner: &mut SafeFileInner) -> io::Result<()> {
    while let Some(pos) = inner.pending.iter().position(|&b| b == b'\n') {
        let raw_line: Vec<u8> = inner.pending.drain(..=pos).collect();
        let lossy = String::from_utf8_lossy(&raw_line);
        let redacted = redact_secrets(&lossy);
        let line = if cfg!(windows) {
            to_crlf(&redacted)
        } else {
            redacted
        };
        inner.file.write_all(line.as_bytes())?;
    }
    Ok(())
}

impl Write for SafeFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log mutex poisoned"))?;
        guard.pending.extend_from_slice(buf);
        // 防止异常超长无换行撑爆内存
        if guard.pending.len() > 256 * 1024 {
            let lossy = String::from_utf8_lossy(&guard.pending);
            let redacted = redact_secrets(&lossy);
            let line = if cfg!(windows) {
                to_crlf(&redacted)
            } else {
                redacted
            };
            guard.file.write_all(line.as_bytes())?;
            guard.pending.clear();
        } else {
            flush_complete_lines(&mut guard)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log mutex poisoned"))?;
        if !guard.pending.is_empty() {
            let lossy = String::from_utf8_lossy(&guard.pending);
            let redacted = redact_secrets(&lossy);
            let line = if cfg!(windows) {
                to_crlf(&redacted)
            } else {
                redacted
            };
            guard.file.write_all(line.as_bytes())?;
            guard.pending.clear();
        }
        guard.file.flush()
    }
}

impl<'a> MakeWriter<'a> for SafeFileWriter {
    type Writer = SafeFileWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// 控制台写入：按行缓冲脱敏（避免 token 跨 write 块漏出）；受 CONSOLE_ENABLED 门闩控制。
#[derive(Clone)]
struct SafeStderr {
    pending: Arc<Mutex<Vec<u8>>>,
}

impl Default for SafeStderr {
    fn default() -> Self {
        Self {
            pending: Arc::new(Mutex::new(Vec::with_capacity(512))),
        }
    }
}

fn flush_stderr_lines(pending: &mut Vec<u8>) -> io::Result<()> {
    while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
        let raw_line: Vec<u8> = pending.drain(..=pos).collect();
        if !CONSOLE_ENABLED.load(Ordering::Relaxed) {
            continue;
        }
        let lossy = String::from_utf8_lossy(&raw_line);
        let redacted = redact_secrets(&lossy);
        let mut out = io::stderr().lock();
        out.write_all(redacted.as_bytes())?;
    }
    Ok(())
}

impl Write for SafeStderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .pending
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log mutex poisoned"))?;
        guard.extend_from_slice(buf);
        if guard.len() > 256 * 1024 {
            let lossy = String::from_utf8_lossy(&guard);
            let redacted = redact_secrets(&lossy);
            guard.clear();
            if CONSOLE_ENABLED.load(Ordering::Relaxed) {
                let mut out = io::stderr().lock();
                out.write_all(redacted.as_bytes())?;
            }
        } else {
            flush_stderr_lines(&mut guard)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .pending
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log mutex poisoned"))?;
        if !guard.is_empty() {
            let lossy = String::from_utf8_lossy(&guard);
            let redacted = redact_secrets(&lossy);
            guard.clear();
            if CONSOLE_ENABLED.load(Ordering::Relaxed) {
                let mut out = io::stderr().lock();
                out.write_all(redacted.as_bytes())?;
            }
        }
        if CONSOLE_ENABLED.load(Ordering::Relaxed) {
            io::stderr().flush()?;
        }
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for SafeStderr {
    type Writer = SafeStderr;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Windows：彻底禁用控制台窗口——永不 AllocConsole，并始终 FreeConsole。
/// `want` 只影响 tracing 是否写 stderr（调试器/管道仍可接到输出），不弹出黑框。
#[cfg(windows)]
pub fn sync_windows_console(_want: bool) {
    #[link(name = "kernel32")]
    extern "system" {
        fn FreeConsole() -> i32;
    }
    unsafe {
        FreeConsole();
    }
}

#[cfg(not(windows))]
pub fn sync_windows_console(_want: bool) {}

/// 初始化全局 subscriber；不可写时关闭文件层，切勿让 appender panic。
pub fn init_logging(resource_root: &Path, cfg: &LogConfig) -> WorkerGuard {
    // 控制台层始终挂载；门闩控制是否写 stderr（不分配控制台窗口）
    let want_console = cfg.console;
    set_console_output(want_console);

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
    let file_enabled = cfg.enabled && log_file.is_some();

    let (nb_writer, guard) = if let Some(ref file) = log_file {
        match SafeFileWriter::open(file) {
            Ok(w) => tracing_appender::non_blocking(w),
            Err(e) => {
                eprintln!("打开日志文件失败 {}: {e}", file.display());
                tracing_appender::non_blocking(io::sink())
            }
        }
    } else {
        tracing_appender::non_blocking(io::sink())
    };

    let file_layer = file_enabled.then(|| {
        fmt::layer()
            .with_writer(nb_writer)
            .with_ansi(false)
            .with_timer(LocalTimer)
            .with_target(true)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .with_level(true)
    });

    // 始终挂载，由 CONSOLE_ENABLED 决定是否落到 stderr（支持热开/热关）
    let console_layer = fmt::layer()
        .with_writer(SafeStderr::default())
        .with_ansi(true)
        .with_timer(LocalTimer)
        .with_target(true);

    // try_init：全局订阅器已存在时返回错误，避免 SetGlobalDefaultError 把进程打崩。
    if let Err(err) = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()
    {
        eprintln!("tracing subscriber already installed: {err}");
    }

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
        let payload = redact_secrets(&payload);
        tracing::error!(%loc, %payload, "panic");
        eprintln!("panic at {loc}: {payload}");
        show_fatal_dialog(
            "StarQuake",
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
