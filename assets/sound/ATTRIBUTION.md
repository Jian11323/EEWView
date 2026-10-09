# 音效素材授权说明

## `srev/` — SREV 整套（用于 JMA 预警 / 震度 / 海啸提示）

| 项 | 内容 |
|----|------|
| 来源 | [kotoho7/scratch-realtime-earthquake-viewer-page](https://github.com/kotoho7/scratch-realtime-earthquake-viewer-page)（经要石 `public/sound/srev` 同名文件拉取） |
| 许可 | **CC BY-SA 2.0** |
| 客户端路由 | `jma_eew` / JMA 相关震度与海啸提示 → 本目录 |
| 备注 | `shindo7.mp3` 与 `shindo6.mp3` 相同（上游无独立 7 档文件时） |

署名示例：

> JMA alert sounds: SREV / scratch-realtime-earthquake-viewer-page (CC BY-SA 2.0).

再分发须保留署名；对改动过的 SREV 衍生音频继续以 CC BY-SA 2.0（或兼容）授权。

## `countdown/` — 地牛 Wake Up! 中文倒计时

| 项 | 内容 |
|----|------|
| 来源 | [地牛 Wake Up!](https://eew.earthquake.tw/) 中文倒计时播报素材（经要石 README 标注、`public/sound/general` 同名文件拉取） |
| 许可 | **非自由许可**；版权归属地牛 Wake Up! 作者。本仓库仅作个人/学习客户端素材引用，**发行前须自行确认是否获授权或改回自有倒计时** |
| 文件 | `0s.mp3`…`10s.mp3`、`20/30/40/50/60s.mp3`、`countdown.wav` |
| 客户端路由 | 本地 S 波倒计时播报 |
| 仓库 | **`assets/sound/countdown/` 已 gitignore**，不进入公开 GitHub，仅本机开发可自备 |

关于页建议注明：

> 倒计时语音素材来自「地牛 Wake Up!」。

## `eew/` · `station/` · `ui/` — Jian Project 原创

由 `tools/synth_sounds.py` 程序合成，**CC0 1.0**。  
用于 **非 JMA** 源（CEA/CENC/CWA 等）的事件提示；**不要**用它们顶替 SREV 的 JMA 流程。

合成方向：近满幅锯齿双音脉冲 + 上行汽笛（对照常见 EEW 客户端警报段的响度/音程/节奏特征），**不包含**对第三方 WAV/人声的采样或重编码。

再生原创包（不含 countdown / srev）：

```bash
python tools/synth_sounds.py
```
