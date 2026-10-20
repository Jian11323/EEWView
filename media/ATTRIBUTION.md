# 地图标记、品牌与烈度图标

## 地图标记 SVG（markers/）

| 文件 | 用途 |
|------|------|
| markers/cross.svg | 预警震中 |
| markers/red.svg | 速报震中（绘制尺寸随震级） |
| markers/green.svg | 本机位置（蓝色房子） |

来源：eewcn（中国地震预警网相关客·端素材）。**··开发者授权**，可在 StarQuake 中使用。  
再分发或商用·自行确认授权范围。

## 品牌（brand/）

| 文件 | 用途 |
|------|------|
| brand/logo.svg | 矢量应用图标（源） |
| brand/logo-wordmark.svg | 字标 |
| brand/favicon.svg | 站点小图标 |
| brand/app.png | 窗口/关于页位图（由 logo.svg 生成） |
| brand/app.ico | Windows exe 图标（由 logo.svg 生成） |
| brand/splash_bg.png | 启动界面背景（实机界面高斯模糊） |

## 烈度 / 震度图标（intensity/）

图标提供：**平平无奇的4**。

运行时由 starquake-ui 嵌入 csis/shindo/mmi/lpgm 及对应 `_inst`（SVG）。

| 目录 | 标度 | 说明 |
|------|------|------|
| intensity/csis/ | 中国烈度（CSIS） | 列表与信息头 |
| intensity/csis_inst/ | 中国仪器烈度 | ·接入 |
| intensity/shindo/ | 气象厅震度 | 列表与信息头；5-/5+ 为 5弱/5强 |
| intensity/shindo_inst/ | 仪器震度 | ·接入 |
| intensity/mmi/ | MMI | ·接入 |
| intensity/mmi_inst/ | MMI 仪器烈度 | ·接入 |
| intensity/lpgm/ | 长周期地震动阶级 | ·接入 |
| intensity/lpgm_inst/ | 仪器长周期阶级 | ·接入 |

## 测站表（stations/）

| 文件 | 用途 |
|------|------|
| stations/nied_point.csv | 原版 NIED kmoni/lmoni 像素坐标 |
| stations/pixels.csv | 原版 MSIL S-Net / 相模湾像素坐标 |

`pixels.csv` 字段：`x,y,type,code,lon,lat`；切片源为·上保安厅 MSIL `smoni`。

## 霞鹜文·（lxgw-wenkai-v1.522/）

| 文件 | 用途 |
|------|------|
| LXGWWenKai-Regular.ttf | 界面主字体（`include_bytes!` 编译嵌入） |
| LXGWWenKaiMono-*.ttf / Light·Medium | 随仓备用（未嵌入，减小二进制与编译时间） |
| OFL.txt | SIL Open Font License 1.1 |

来源：[lxgw/LxgwWenKai](https://github.com/lxgw/LxgwWenKai) v1.522。**SIL OFL 1.1**，可免费开源商用；再分发须保留许可证与著作权声明。运行时不依赖外置字体文件。

## 音效（sound/）

全部音频文件平铺于 `media/sound/` 根目录（无子目录）。逻辑音效包由 `catalog.json` 的 `packs` 区分。路由说明见 [`docs/音效路由.md`](../docs/音效路由.md)。

### SREV

| 项 | 内容 |
|----|------|
| 来源 | [kotoho7/scratch-realtime-earthquake-viewer-page](https://github.com/kotoho7/scratch-realtime-earthquake-viewer-page) |
| 许可 | **CC BY-SA 2.0** |
| 用途 | 预警发行 / 更新 / 震度 / 海啸（`issue.mp3` / `caution.mp3` / `warn.mp3` / `update.mp3` / `final.mp3` / `cancel.mp3` / `shindo*.mp3` / `tsunami*.mp3` 等） |

署名示例：

> JMA alert sounds: SREV / scratch-realtime-earthquake-viewer-page (CC BY-SA 2.0).

### Open EEW CN

| 项 | 内容 |
|----|------|
| 来源 | [OpenEEWCN/eewcn](https://github.com/OpenEEWCN/eewcn) `Media/` |
| 许可 | 开发者授权用于 StarQuake；再分发请自行确认 |
| 用途 | 非 JMA 预警 / 速报 / 本机 S 波抵达 |

| 文件 | 用途 |
|------|------|
| `eewalert.wav` | 预警：烈度 < 7 且震级 < M5 |
| `eewcritical.wav` | 预警：烈度 ≥ 7 或震级 ≥ M5 |
| `eewupdate.wav` | 预警更新 / 最终报（未达危急档时） |
| `newrecord.wav` | 速报 |
| `swavearrive.wav` | 本机 S 波抵达 |

### 地牛 Wake Up! 中文倒计时

| 项 | 内容 |
|----|------|
| 来源 | [地牛 Wake Up!](https://eew.earthquake.tw/) |
| 许可 | **非自由许可**；版权归属原作者。公开再分发前须确认授权或更换素材 |
| 用途 | 本地 S 波倒计时播报 |
| 文件 | `countdown.wav`、`0s.mp3`…`10s.mp3`、`20s.mp3`…`60s.mp3` |
| 仓库 | 上述文件已加入 `.gitignore`，不随公开仓库分发 |

### 原创合成（回退 / 测站 / UI）

由 `tools/synth_sounds.py` 合成，**CC0 1.0**。

| 文件 | 用途 |
|------|------|
| `issue.wav` / `update.wav` / `final.wav` / `cancel.wav` / `prompt.wav` | EEW 回退 |
| `arrive_p.wav` / `arrive_s.wav` | P/S 波抵达回退 |
| `weak.wav` / `mid.wav` / `strong.wav` | 测站 |
| `connect_ok.wav` / `connect_fail.wav` / `notify.wav` | UI |


## 自定义告警音（special/）

| 文件 | 用途 |
|------|------|
| `nhk_news_bell.wav`（源自 `NHK一級ニュースベル.wav`） | 情报/预警震度 ≥6 弱 |
| `nhk_eew_alert.wav`（源自 `NHK緊急地震速報の音.wav`） | JMA 紧急地震速报「警报」 |
| `ebs_type2_start.wav`（源自 `緊急警報放送 第 2 種開始信号.wav`） | 津波警报 / 大津波警报 |

上述为广播/公开试听类素材副本，再分发前请自行确认授权。
