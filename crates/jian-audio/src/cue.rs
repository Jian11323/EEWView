//! 业务侧发出的播报意图（由 core/app 决定「要不要响」，audio 只负责播）。

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCue {
    /// 首报 / 一般发行
    Issue,
    /// 后续报更新
    Update,
    /// 最终报
    Final,
    /// 取消报
    Cancel,
    /// 震度档语音（JMA 0–7 展示档，映射 catalog shindo_*）
    Shindo(u8),
    /// 本地 S 波剩余整秒播报（0–60）
    Countdown(i32),
    /// 倒计时循环底噪开/关
    CountdownLoop(bool),
    /// UI：连接成功 / 失败 / 通知
    UiConnectOk,
    UiConnectFail,
    UiNotify,
}
