//! Windows SAPI TTS（后台线程；COM ISpVoice，不经 PowerShell / 控制台）。
//! 非 Windows 为 no-op。

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
            #[cfg(target_os = "windows")]
            {
                let _com = match ComApartment::enter() {
                    Ok(g) => g,
                    Err(e) => {
                        warn!("tts: COM init failed: {e}");
                        return;
                    }
                };
                let voice = match create_voice() {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("tts: SpVoice create failed: {e}");
                        return;
                    }
                };
                while let Ok(text) = rx.recv() {
                    let mut latest = text;
                    while let Ok(more) = rx.try_recv() {
                        if !more.trim().is_empty() {
                            latest = more;
                        }
                    }
                    if latest.trim().is_empty() {
                        continue;
                    }
                    speak_blocking(&voice, &latest);
                }
            }
            #[cfg(not(target_os = "windows"))]
            while let Ok(text) = rx.recv() {
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
struct ComApartment;

#[cfg(target_os = "windows")]
impl ComApartment {
    fn enter() -> windows::core::Result<Self> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        // S_FALSE = 本线程已初始化，仍可继续用
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

#[cfg(target_os = "windows")]
impl Drop for ComApartment {
    fn drop(&mut self) {
        use windows::Win32::System::Com::CoUninitialize;
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(target_os = "windows")]
fn create_voice() -> windows::core::Result<windows::Win32::Media::Speech::ISpVoice> {
    use windows::Win32::Media::Speech::SpVoice;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) }
}

#[cfg(target_os = "windows")]
fn speak_blocking(voice: &windows::Win32::Media::Speech::ISpVoice, text: &str) {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK};

    // 纯文本、先清未念完的旧句；同步说完再返回（与原先 PowerShell Speak 一致）
    let flags = (SPF_IS_NOT_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32;
    let hs = HSTRING::from(text);
    if let Err(e) = unsafe { voice.Speak(&hs, flags, None) } {
        warn!("tts: Speak failed: {e}");
    }
}

#[cfg(not(target_os = "windows"))]
fn speak_blocking(text: &str) {
    debug!(%text, "tts: non-windows no-op");
}
