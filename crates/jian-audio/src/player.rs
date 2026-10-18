//! rodio 输出：事件轨 + 倒计时循环轨。

use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tracing::{info, warn};

pub struct Player {
    _stream: OutputStream,
    _handle: OutputStreamHandle,
    event_sink: Sink,
    loop_sink: Sink,
    /// 倒计时语音，不打断事件音
    speech_sink: Sink,
    master: f32,
    countdown_vol: f32,
    mute: bool,
}

impl Player {
    pub fn try_open(master: f32, countdown_vol: f32, mute: bool) -> Result<Self> {
        let (stream, handle) =
            OutputStream::try_default().context("打开默认音频输出失败（无声卡/权限？）")?;
        let event_sink = Sink::try_new(&handle).context("event sink")?;
        let loop_sink = Sink::try_new(&handle).context("loop sink")?;
        let speech_sink = Sink::try_new(&handle).context("speech sink")?;
        let player = Self {
            _stream: stream,
            _handle: handle,
            event_sink,
            loop_sink,
            speech_sink,
            master: master.clamp(0.0, 1.0),
            countdown_vol: countdown_vol.clamp(0.0, 1.0),
            mute,
        };
        player.apply_volumes();
        info!("audio output ready");
        Ok(player)
    }

    pub fn set_mute(&mut self, mute: bool) {
        self.mute = mute;
        self.apply_volumes();
    }

    pub fn set_volumes(&mut self, master: f32, countdown_vol: f32) {
        self.master = master.clamp(0.0, 1.0);
        self.countdown_vol = countdown_vol.clamp(0.0, 1.0);
        self.apply_volumes();
    }

    fn apply_volumes(&self) {
        if self.mute {
            self.event_sink.set_volume(0.0);
            self.loop_sink.set_volume(0.0);
            self.speech_sink.set_volume(0.0);
        } else {
            self.event_sink.set_volume(self.master);
            self.loop_sink.set_volume(self.countdown_vol);
            self.speech_sink.set_volume(self.countdown_vol);
        }
    }

    /// 清空事件轨队列后播一次
    pub fn play_oneshot(&self, path: &Path) {
        self.event_sink.stop();
        match decode(path) {
            Ok(source) => {
                self.event_sink.append(source);
                self.event_sink.play();
            }
            Err(e) => warn!(path = %path.display(), "play oneshot: {e:#}"),
        }
    }

    /// 倒计时语音（地牛 10s/9s…），不占用事件轨
    pub fn play_speech(&self, path: &Path) {
        self.speech_sink.stop();
        match decode(path) {
            Ok(source) => {
                self.speech_sink.append(source);
                self.speech_sink.play();
            }
            Err(e) => warn!(path = %path.display(), "countdown speech: {e:#}"),
        }
    }

    /// `Some(path)` 循环；`None` 停止
    pub fn set_countdown_loop(&self, path: Option<&Path>) {
        self.loop_sink.stop();
        let Some(path) = path else {
            return;
        };
        match decode(path) {
            Ok(source) => {
                self.loop_sink.append(source.repeat_infinite());
                self.loop_sink.play();
            }
            Err(e) => warn!(path = %path.display(), "countdown loop: {e:#}"),
        }
    }
}

fn decode(path: &Path) -> Result<Decoder<BufReader<File>>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    Decoder::new(BufReader::new(file)).context("decode audio")
}
