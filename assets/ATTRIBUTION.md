# 地图标记、品牌与烈度图标

## 地图标记 SVG（markers/）

| 文件 | 用途 |
|------|------|
| markers/cross.svg | 预警震中 |
| markers/red.svg | 速报震中（绘制尺寸随震级） |
| markers/green.svg | 本机位置（蓝色房子） |

来源：eewcn（中国地震预警网相关客·端素材）。**··开发者授权**，可在 EEWView 中使用。  
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

运行时由 jian-ui 嵌入 csis/shindo/mmi/lpgm 及对应 `_inst`（SVG）。

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
