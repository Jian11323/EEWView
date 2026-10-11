//! 本地 S 波倒计时播报状态机：整十秒 + ≤10 个位 + 末 30 秒循环底噪。

use crate::cue::AudioCue;

/// 循环底噪仅在剩余 ≤ 此秒数时开启（避免远震长时间滴滴）。
pub const LOOP_WITHIN_SECS: i32 = 30;

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
    /// 上一帧剩余整秒；用于秒边界重启滴滴，与 HUD 对齐
    last_sec: Option<i32>,
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
            self.last_sec = None;
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
            self.last_sec = Some(sec);
            return out;
        }

        let sec_changed = self.last_sec != Some(sec);

        // 仅末 30 秒开循环底噪；更早只报整十秒语音
        // 每一整秒边界重启循环，避免自由播放与 HUD 秒数漂移
        let want_loop = sec <= LOOP_WITHIN_SECS;
        if want_loop {
            if !self.loop_on || sec_changed {
                out.push(AudioCue::CountdownLoop(true));
                self.loop_on = true;
            }
        } else if self.loop_on {
            out.push(AudioCue::CountdownLoop(false));
            self.loop_on = false;
        }

        if let Some(a) = announce_sec(sec) {
            if self.last_announced != Some(a) {
                out.push(AudioCue::Countdown(a));
                self.last_announced = Some(a);
            }
        }

        self.last_sec = Some(sec);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_stays_off_above_30s() {
        let mut m = CountdownMachine::default();
        let cues = m.tick(Some(120));
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(_))));
        let cues = m.tick(Some(45));
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(_))));
    }

    #[test]
    fn loop_starts_at_30s() {
        let mut m = CountdownMachine::default();
        let _ = m.tick(Some(40));
        let cues = m.tick(Some(30));
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        // 同秒内不重复开循环
        let cues = m.tick(Some(30));
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        // 秒变了：重启循环以对齐 HUD
        let cues = m.tick(Some(29));
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
    }

    #[test]
    fn loop_stops_when_cleared() {
        let mut m = CountdownMachine::default();
        let cues = m.tick(Some(20));
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        let cues = m.tick(None);
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(false))));
    }
}
