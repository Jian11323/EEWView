# 地图标记、品牌与烈度图标

## 地图标记 SVG（markers/）

| 文件 | 用途 |
|------|------|
| markers/cross.svg | 预警震中 |
| markers/red.svg | 速报震中（绘制尺寸随震级） |
| markers/green.svg | 本机位置 |

来源：eewcn（中国地震预警网相关客户端素材）。**已获开发者授权**，可在 EEWView 中使用。  
再分发或商用请自行确认授权范围。

## 品牌（rand/）

| 文件 | 用途 |
|------|------|
| rand/logo.svg | 应用图标 |
| rand/logo-wordmark.svg | 字标 |
| rand/favicon.svg | 站点/窗口图标 |

## 烈度 / 震度图标（intensity/）

运行时由 jian-ui 嵌入 csis/ 与 shindo/（SVG）；其余标度预留。

| 目录 | 标度 | 说明 |
|------|------|------|
| intensity/csis/ | 中国烈度（CSIS） | 列表与信息头 |
| intensity/csis_inst/ | 中国仪器烈度 | 预留 |
| intensity/shindo/ | 气象厅震度 | 列表与信息头；5-/5+ 为 5弱/5强 |
| intensity/shindo_inst/ | 仪器震度 | 预留 |
| intensity/mmi/ | MMI | 预留 |
| intensity/mmi_inst/ | MMI 仪器烈度 | 预留 |
| intensity/lpgm/ | 长周期地震动阶级 | 预留 |
| intensity/lpgm_inst/ | 仪器长周期阶级 | 预留 |
