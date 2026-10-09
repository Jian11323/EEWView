//! 本地 S 波倒计时播报状态机：整十秒 + ≤10 个位 + 循环底噪。

use crate::cue::AudioCue;

/// 应播报的秒档：60/50/…/20、10…0；其它秒静默。
pub fn announce_sec(remain: i32) -> Option<i32> {
    if remain < 0 {
        return None;
    }
    if remain <= 10 {
        return Some(remain);
    }
    if remain <= 60 && remain % 10 == 0 {
        return Some(remain);
    }
    None
}

#[derive(Debug, Default)]
pub struct CountdownMachine {
    last_announced: Option<i32>,
    loop_on: bool,
    active: bool,
}

impl CountdownMachine {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 每帧喂入当前剩余秒；返回需要立刻播放的 cue。
    pub fn tick(&mut self, remain: Option<i32>) -> Vec<AudioCue> {
        let mut out = Vec::new();
        let Some(sec) = remain else {
            if self.loop_on {
                out.push(AudioCue::CountdownLoop(false));
            }
            self.reset();
            return out;
        };

        if !self.active {
            self.active = true;
            self.last_announced = None;
            if sec > 0 {
                out.push(AudioCue::CountdownLoop(true));
                self.loop_on = true;
            }
        }

        if sec <= 0 {
            if self.last_announced != Some(0) {
                out.push(AudioCue::Countdown(0));
                self.last_announced = Some(0);
            }
            if self.loop_on {
                out.push(AudioCue::CountdownLoop(false));
                self.loop_on = false;
            }
            return out;
        }

        if let Some(a) = announce_sec(sec) {
            if self.last_announced != Some(a) {
                out.push(AudioCue::Countdown(a));
                self.last_announced = Some(a);
            }
        }

        out
    }
}
