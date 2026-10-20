//! 本地 S 波倒计时播报（对齐要石）：从 `countdown_start` 起播地牛语音，并循环滴答。

use crate::cue::AudioCue;

/// 应播报的秒档。`start_at` 以内：≤10 每秒，更早仅整十秒（有对应语音文件）。
pub fn announce_sec(remain: i32, start_at: i32) -> Option<i32> {
    if remain < 0 || remain > start_at.max(0) {
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

    /// 每帧喂入当前剩余秒。`speech=false` 时只留滴答。
    pub fn tick(&mut self, remain: Option<i32>, start_at: i32, speech: bool) -> Vec<AudioCue> {
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

        // 从设定秒数起循环滴答；每一整秒边界重启，与 HUD 对齐
        let want_loop = sec <= start_at.max(0);
        if want_loop {
            if !self.loop_on || sec_changed {
                out.push(AudioCue::CountdownLoop(true));
                self.loop_on = true;
            }
        } else if self.loop_on {
            out.push(AudioCue::CountdownLoop(false));
            self.loop_on = false;
        }

        if speech {
            if let Some(a) = announce_sec(sec, start_at) {
                if self.last_announced != Some(a) {
                    out.push(AudioCue::Countdown(a));
                    self.last_announced = Some(a);
                }
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
    fn loop_stays_off_above_start() {
        let mut m = CountdownMachine::default();
        let cues = m.tick(Some(120), 10, true);
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(_))));
        let cues = m.tick(Some(45), 10, true);
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(_))));
    }

    #[test]
    fn speech_and_loop_start_at_ten() {
        let mut m = CountdownMachine::default();
        let _ = m.tick(Some(12), 10, true);
        let cues = m.tick(Some(10), 10, true);
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        assert!(cues.iter().any(|c| matches!(c, AudioCue::Countdown(10))));
        let cues = m.tick(Some(10), 10, true);
        assert!(!cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        let cues = m.tick(Some(9), 10, true);
        assert!(cues.iter().any(|c| matches!(c, AudioCue::Countdown(9))));
    }

    #[test]
    fn loop_stops_when_cleared() {
        let mut m = CountdownMachine::default();
        let cues = m.tick(Some(8), 10, true);
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(true))));
        let cues = m.tick(None, 10, true);
        assert!(cues.iter().any(|c| matches!(c, AudioCue::CountdownLoop(false))));
    }
}
