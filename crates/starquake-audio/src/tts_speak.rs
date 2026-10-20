//! TTS：离线系统语音包（Windows SAPI）与微软 Edge 在线 TTS。
//! 后台线程串行朗读；COM / Edge 合成均不经 PowerShell。

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use rodio::{Decoder, OutputStream, Sink};
use tracing::{debug, warn};

/// TTS 引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtsEngine {
    /// Windows 已安装的离线 SAPI 语音包
    Offline,
    /// 微软 Edge 在线 Neural TTS
    Edge,
}

impl TtsEngine {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "edge" | "online" | "msedge" => Self::Edge,
            _ => Self::Offline,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Edge => "edge",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TtsVoiceInfo {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct TtsRuntimeConfig {
    pub engine: TtsEngine,
    /// SAPI 语音名；空=默认
    pub offline_voice: String,
    /// Edge short name，如 `zh-CN-XiaoxiaoNeural`
    pub edge_voice: String,
    /// 0.5–2.0，1.0 为默认
    pub rate: f32,
    /// 0–1，与主音量叠加感
    pub volume: f32,
}

impl Default for TtsRuntimeConfig {
    fn default() -> Self {
        Self {
            engine: TtsEngine::Offline,
            offline_voice: String::new(),
            edge_voice: "zh-CN-XiaoxiaoNeural".into(),
            rate: 1.0,
            volume: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
struct TtsJob {
    text: String,
    cfg: TtsRuntimeConfig,
}

static TTS_TX: Mutex<Option<Sender<TtsJob>>> = Mutex::new(None);
static TTS_CFG: Mutex<TtsRuntimeConfig> = Mutex::new(TtsRuntimeConfig {
    engine: TtsEngine::Offline,
    offline_voice: String::new(),
    edge_voice: String::new(), // configure_tts / Default 会补齐晓晓
    rate: 1.0,
    volume: 1.0,
});
static TTS_GEN: AtomicU64 = AtomicU64::new(0);

fn snap_cfg() -> TtsRuntimeConfig {
    TTS_CFG
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn normalize_cfg(cfg: TtsRuntimeConfig) -> TtsRuntimeConfig {
    TtsRuntimeConfig {
        engine: cfg.engine,
        offline_voice: cfg.offline_voice,
        edge_voice: if cfg.edge_voice.trim().is_empty() {
            "zh-CN-XiaoxiaoNeural".into()
        } else {
            cfg.edge_voice
        },
        rate: cfg.rate.clamp(0.5, 2.0),
        volume: cfg.volume.clamp(0.0, 1.0),
    }
}

/// 热更新 TTS 运行参数（设置保存 / 试听前调用）。
pub fn configure_tts(cfg: TtsRuntimeConfig) {
    if let Ok(mut g) = TTS_CFG.lock() {
        *g = normalize_cfg(cfg);
    }
    // 打断正在播的在线合成
    TTS_GEN.fetch_add(1, Ordering::SeqCst);
}

fn ensure_worker() -> Option<Sender<TtsJob>> {
    let mut guard = TTS_TX.lock().ok()?;
    if let Some(tx) = guard.as_ref() {
        return Some(tx.clone());
    }
    let (tx, rx) = mpsc::channel::<TtsJob>();
    thread::Builder::new()
        .name("starquake-tts".into())
        .spawn(move || {
            let mut edge_out: Option<(OutputStream, Sink)> = None;
            while let Ok(job) = rx.recv() {
                let mut latest = job;
                while let Ok(more) = rx.try_recv() {
                    if !more.text.trim().is_empty() {
                        latest = more;
                    }
                }
                if latest.text.trim().is_empty() {
                    continue;
                }
                let cfg = latest.cfg;
                let text = latest.text;
                let gen = TTS_GEN.load(Ordering::SeqCst);
                match cfg.engine {
                    TtsEngine::Offline => {
                        #[cfg(target_os = "windows")]
                        speak_offline_blocking(&text, &cfg);
                        #[cfg(not(target_os = "windows"))]
                        {
                            let _ = (&text, &cfg);
                            debug!("tts offline: non-windows no-op");
                        }
                    }
                    TtsEngine::Edge => {
                        if gen != TTS_GEN.load(Ordering::SeqCst) {
                            continue;
                        }
                        if edge_out.is_none() {
                            match OutputStream::try_default() {
                                Ok((stream, handle)) => match Sink::try_new(&handle) {
                                    Ok(sink) => edge_out = Some((stream, sink)),
                                    Err(e) => {
                                        warn!("tts edge: sink failed: {e}");
                                        continue;
                                    }
                                },
                                Err(e) => {
                                    warn!("tts edge: output failed: {e}");
                                    continue;
                                }
                            }
                        }
                        if let Some((_, sink)) = edge_out.as_ref() {
                            speak_edge_blocking(&text, &cfg, sink, gen);
                        }
                    }
                }
            }
        })
        .ok()?;
    *guard = Some(tx.clone());
    Some(tx)
}

/// 异步朗读（入队时快照当前配置；新句会挤掉未开播的旧句）。
pub fn speak_async(text: impl Into<String>) {
    speak_with_config(text, snap_cfg());
}

/// 用指定配置朗读（试听草稿，不改动默认运行参数）。
pub fn speak_with_config(text: impl Into<String>, cfg: TtsRuntimeConfig) {
    let text = text.into();
    if text.trim().is_empty() {
        return;
    }
    // 打断正在进行的 Edge 播放；SAPI 侧靠队列串行
    TTS_GEN.fetch_add(1, Ordering::SeqCst);
    let job = TtsJob {
        text,
        cfg: normalize_cfg(cfg),
    };
    match ensure_worker() {
        Some(tx) => {
            if tx.send(job).is_err() {
                warn!("tts: worker channel closed");
            }
        }
        None => debug!("tts: worker unavailable"),
    }
}

/// 常用 Edge 中文神经音色（离线列表，无需联网拉取）。
pub fn edge_voice_presets() -> Vec<TtsVoiceInfo> {
    const PRESETS: &[(&str, &str)] = &[
        ("zh-CN-XiaoxiaoNeural", "晓晓（女声）"),
        ("zh-CN-YunxiNeural", "云希（男声）"),
        ("zh-CN-YunyangNeural", "云扬（男声）"),
        ("zh-CN-XiaoyiNeural", "晓伊（女声）"),
        ("zh-CN-YunjianNeural", "云健（男声）"),
        ("zh-CN-XiaochenNeural", "晓辰（女声）"),
        ("zh-CN-XiaohanNeural", "晓涵（女声）"),
        ("zh-CN-XiaomengNeural", "晓梦（女声）"),
        ("zh-CN-XiaomoNeural", "晓墨（女声）"),
        ("zh-CN-XiaoruiNeural", "晓睿（女声）"),
        ("zh-CN-XiaoshuangNeural", "晓双（女声）"),
        ("zh-CN-XiaoxuanNeural", "晓萱（女声）"),
        ("zh-CN-XiaoyanNeural", "晓颜（女声）"),
        ("zh-CN-XiaoyouNeural", "晓悠（女声）"),
        ("zh-CN-YunfengNeural", "云枫（男声）"),
        ("zh-CN-YunhaoNeural", "云皓（男声）"),
        ("zh-CN-YunxiaNeural", "云夏（男声）"),
        ("zh-CN-YunyeNeural", "云野（男声）"),
        ("zh-CN-YunzeNeural", "云泽（男声）"),
        ("zh-TW-HsiaoChenNeural", "曉臻（台湾）"),
        ("zh-TW-HsiaoYuNeural", "曉雨（台湾）"),
        ("zh-HK-HiuMaanNeural", "曉曼（粤语）"),
    ];
    PRESETS
        .iter()
        .map(|(id, label)| TtsVoiceInfo {
            id: (*id).into(),
            label: (*label).into(),
        })
        .collect()
}

/// 枚举本机已安装的离线 SAPI 语音包。
pub fn list_offline_voices() -> Vec<TtsVoiceInfo> {
    let mut out = vec![TtsVoiceInfo {
        id: String::new(),
        label: "系统默认".into(),
    }];
    #[cfg(target_os = "windows")]
    {
        match enum_sapi_voices() {
            Ok(mut list) => {
                list.sort_by(|a, b| a.label.cmp(&b.label));
                out.extend(list);
            }
            Err(e) => warn!("tts: list offline voices failed: {e}"),
        }
    }
    out
}

fn rate_to_percent(rate: f32) -> i32 {
    (((rate.clamp(0.5, 2.0) - 1.0) * 100.0).round() as i32).clamp(-50, 100)
}

fn rate_to_sapi(rate: f32) -> i32 {
    (((rate.clamp(0.5, 2.0) - 1.0) * 10.0).round() as i32).clamp(-10, 10)
}

fn speak_edge_blocking(text: &str, cfg: &TtsRuntimeConfig, sink: &Sink, gen: u64) {
    use msedge_tts::tts::client::connect;
    use msedge_tts::tts::SpeechConfig;

    if gen != TTS_GEN.load(Ordering::SeqCst) {
        return;
    }

    let voice = if cfg.edge_voice.trim().is_empty() {
        "zh-CN-XiaoxiaoNeural"
    } else {
        cfg.edge_voice.trim()
    };
    let speech = SpeechConfig {
        voice_name: voice.into(),
        audio_format: "audio-24khz-48kbitrate-mono-mp3".into(),
        pitch: 0,
        rate: rate_to_percent(cfg.rate),
        volume: ((cfg.volume.clamp(0.0, 1.0) * 100.0).round() as i32 - 100).clamp(-50, 100),
    };

    let audio = match connect().and_then(|mut c| c.synthesize(text, &speech)) {
        Ok(a) => a,
        Err(e) => {
            warn!("tts edge: synthesize failed: {e}");
            return;
        }
    };
    if gen != TTS_GEN.load(Ordering::SeqCst) {
        return;
    }
    let bytes = audio.audio_bytes;
    if bytes.is_empty() {
        warn!("tts edge: empty audio");
        return;
    }

    sink.stop();
    match Decoder::new(Cursor::new(bytes)) {
        Ok(source) => {
            sink.set_volume(cfg.volume.clamp(0.0, 1.0));
            sink.append(source);
            sink.play();
            // 等到播完或被新配置打断
            while !sink.empty() {
                if gen != TTS_GEN.load(Ordering::SeqCst) {
                    sink.stop();
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
        }
        Err(e) => warn!("tts edge: decode failed: {e}"),
    }
}

#[cfg(target_os = "windows")]
struct ComApartment;

#[cfg(target_os = "windows")]
impl ComApartment {
    fn enter() -> windows::core::Result<Self> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
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
fn speak_offline_blocking(text: &str, cfg: &TtsRuntimeConfig) {
    use windows::core::HSTRING;
    use windows::Win32::Media::Speech::{SPF_IS_NOT_XML, SPF_PURGEBEFORESPEAK};

    let _com = match ComApartment::enter() {
        Ok(g) => g,
        Err(e) => {
            warn!("tts offline: COM init failed: {e}");
            return;
        }
    };
    let voice = match create_voice() {
        Ok(v) => v,
        Err(e) => {
            warn!("tts offline: SpVoice create failed: {e}");
            return;
        }
    };

    if let Err(e) = unsafe { voice.SetRate(rate_to_sapi(cfg.rate)) } {
        debug!("tts offline: SetRate: {e}");
    }
    let vol = (cfg.volume.clamp(0.0, 1.0) * 100.0).round() as u16;
    if let Err(e) = unsafe { voice.SetVolume(vol) } {
        debug!("tts offline: SetVolume: {e}");
    }
    if let Err(e) = apply_sapi_voice(&voice, &cfg.offline_voice) {
        debug!("tts offline: SetVoice: {e}");
    }

    let flags = (SPF_IS_NOT_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32;
    let hs = HSTRING::from(text);
    if let Err(e) = unsafe { voice.Speak(&hs, flags, None) } {
        warn!("tts offline: Speak failed: {e}");
    }
}

#[cfg(target_os = "windows")]
fn apply_sapi_voice(
    voice: &windows::Win32::Media::Speech::ISpVoice,
    wanted: &str,
) -> windows::core::Result<()> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Ok(());
    }
    let tokens = enum_sapi_tokens()?;
    for (name, token) in tokens {
        if name.eq_ignore_ascii_case(wanted) {
            unsafe {
                voice.SetVoice(&token)?;
            }
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn enum_sapi_voices() -> windows::core::Result<Vec<TtsVoiceInfo>> {
    let _com = ComApartment::enter()?;
    let tokens = enum_sapi_tokens()?;
    Ok(tokens
        .into_iter()
        .map(|(name, _)| TtsVoiceInfo {
            id: name.clone(),
            label: name,
        })
        .collect())
}

#[cfg(target_os = "windows")]
fn enum_sapi_tokens(
) -> windows::core::Result<Vec<(String, windows::Win32::Media::Speech::ISpObjectToken)>> {
    use windows::core::w;
    use windows::Win32::Media::Speech::{
        ISpObjectToken, ISpObjectTokenCategory, SpObjectTokenCategory, SPCAT_VOICES,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

    let cat: ISpObjectTokenCategory =
        unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)? };
    unsafe {
        cat.SetId(SPCAT_VOICES, false)?;
    }

    let enumerator = unsafe { cat.EnumTokens(None, None)? };
    let mut count = 0u32;
    unsafe {
        enumerator.GetCount(&mut count)?;
    }

    let mut out = Vec::new();
    for i in 0..count {
        let token: ISpObjectToken = match unsafe { enumerator.Item(i) } {
            Ok(t) => t,
            Err(_) => continue,
        };
        let name = match unsafe { token.GetStringValue(w!("Name")) } {
            Ok(pw) => {
                let s = unsafe { pw.to_string().unwrap_or_default() };
                unsafe {
                    CoTaskMemFree(Some(pw.as_ptr() as *const _));
                }
                s
            }
            Err(_) => String::new(),
        };
        if !name.is_empty() {
            out.push((name, token));
        }
    }
    Ok(out)
}
