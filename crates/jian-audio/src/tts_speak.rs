//! Windows SAPI TTS（后台线程；非 Windows 为 no-op）。

use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use tracing::{debug, warn};

static TTS_TX: Mutex<Option<Sender<String>>> = Mutex::new(None);

fn ensure_worker() -> Option<Sender<String>> {
    let mut guard = TTS_TX.lock().ok()?;
    if let Some(tx) = guard.as_ref() {
        return Some(tx.clone());
    }
    let (tx, rx) = mpsc::channel::<String>();
    thread::Builder::new()
        .name("jian-tts".into())
        .spawn(move || {
            while let Ok(text) = rx.recv() {
                // 只念队列里最新一条，丢掉积压（预警多源连刷时否则会念很久）
                let mut latest = text;
                while let Ok(more) = rx.try_recv() {
                    if !more.trim().is_empty() {
                        latest = more;
                    }
                }
                if latest.trim().is_empty() {
                    continue;
                }
                speak_blocking(&latest);
            }
        })
        .ok()?;
    *guard = Some(tx.clone());
    Some(tx)
}

/// 异步朗读中文文案（队列串行，避免叠音）。
pub fn speak_async(text: impl Into<String>) {
    let text = text.into();
    if text.trim().is_empty() {
        return;
    }
    match ensure_worker() {
        Some(tx) => {
            if tx.send(text).is_err() {
                warn!("tts: worker channel closed");
            }
        }
        None => debug!("tts: worker unavailable"),
    }
}

#[cfg(target_os = "windows")]
fn speak_blocking(text: &str) {
    // PowerShell System.Speech；stdin 传 UTF-8，避免命令行代码页弄坏中文
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = r#"
$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::UTF8
[Console]::InputEncoding = [Text.UTF8Encoding]::UTF8
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$s.Rate = 0
$t = [Console]::In.ReadToEnd()
if (-not [string]::IsNullOrWhiteSpace($t)) { $s.Speak($t) }
"#;
    let mut child = match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("tts spawn: {e}");
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
}

#[cfg(not(target_os = "windows"))]
fn speak_blocking(text: &str) {
    debug!(%text, "tts: non-windows no-op");
}
