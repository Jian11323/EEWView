<p align="center">
  <img src="icon/brand/app.png" alt="StarQuake" width="128" />
</p>

<h1 align="center">StarQuake</h1>

<p align="center">
  纯 Rust 桌面端 · 多源预警 / 速报视监
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="License" /></a>
  <img src="https://img.shields.io/badge/Rust-1.70%2B-orange.svg" alt="Rust" />
  <img src="https://img.shields.io/badge/Platform-Windows-0078D6.svg" alt="Windows" />
  <img src="https://img.shields.io/badge/Release-2.0.0-informational.svg" alt="2.0.0" />
</p>

<p align="center">
  <img src="https://socialify.git.ci/Jian11323/StarQuake/image?description=1&amp;font=Inter&amp;language=1&amp;name=1&amp;owner=1&amp;pattern=Solid&amp;theme=Auto" alt="StarQuake" width="640" />
</p>

---

## 简介

**StarQuake** 是一款面向桌面的地震预警与速报视监客户端。  
聚合多源实时情报，在地图上展示震中、走时波圈与测站烈度，并提供本机倒计时与音效播报。

---

## 主要功能

- 接收中国地震局、台湾中央气象署、日本气象厅、韩国气象厅等地震预警
- 接收中国地震台网、日本气象厅等地震速报 / 列表
- 接收日本气象厅海啸情报
- 展示 NIED 強震モニタ、S-Net、KMA 等测站烈度（列表与地图色点）
- 地图叠加震中、P/S 波圈（近距 JMA2001，远距 ak135）
- 暗色矢量底图 / 亮色海洋地形瓦片，可随主题切换
- 台风路径、风圈与多源预报（可开关）
- 地图晨昏线（设置 → 地图显示，默认关）；设置本机经纬度后在右下角显示预估日出与日落
- 配置本机经纬度（地图选点或按公网 IP 定位）后估算本地 S 波倒计时，并支持音效播报
- 各数据源健康状态：**正常** / **波动** / **异常**
- 界面语言切换：简体中文 / 繁体中文 / 日本語 / English（设置 → 通用）
- 右侧情报侧栏可开关（设置、向右拖关闭或右缘拉开）；关闭后地图右键可打开设置、跳转所在地、图例、设置所在地与关于
- 主窗内嵌设置（数据源、地图与显示、音频、性能、高级、关于）
- 自定义告警音：NHK 一级新闻铃、JMA 警报音、津波紧急警报放送；可开关
- 桌面系统通知（预警 / 速报 / 海啸，可分别开关）

---

## 数据来源

- 地震预警（CEA/CWA/JMA/KMA 等）、地震速报、海啸情报、实时测站、台风路径：[Jian Project API](https://api.sismotide.top/api/)
- 地震预警（CEA/SC/FJ/CWA/JMA/CENC）、地震列表（CENC/JMA）：[Wolfx Open API](https://wolfx.jp/api/)（请注意参考接口文档）
- 地震信息（JMA）：[P2PQuake](https://www.p2pquake.net/)
- 地震信息（FSSN）：[FSSN WebSocket API](https://fssn-api.fanstudio.tech/#fssn)
- GlobalQuake 全球预警、JMA-Atom 火山情报：[PancakesAPI](https://wiki.aloys23.link/wiki/PancakesAPI)
- 日本测站烈度：[NIED 強震モニタ](https://www.lmoni.bosai.go.jp/)
- 亮色海洋地形瓦片：[sismotide tilemap](https://tilemap.sismotide.top/)
- IP 归属定位：[sismotide GET IP](https://api.sismotide.top/get/ip.php)
- 中国大陆地图：[阿里云 DataV.GeoAtlas](https://datav.aliyun.com/portal/school/atlas/area_selector)
- 中国台湾地图：[GeoJSON.cn](https://geojson.cn/)
- 日本地图：[日本气象厅](https://www.data.jma.go.jp/developer/gis.html)（注意钓鱼岛地区处理）
- 世界地图：[GeoJSON Maps of the globe](https://geojson-maps.kyd.au/)（注意甄别争议地区）
- 全球活动断层：[GEM Global Active Faults](https://github.com/GEMScienceTools/gem-global-active-faults)
- 中国活动断层：[gmt-china/china-geospatial-data](https://github.com/gmt-china/china-geospatial-data)（CAFD）
- 近距走时表：[气象厅 JMA2001](https://www.data.jma.go.jp/eqev/data/bulletin/catalog/appendix/trtime/trt_j.html)
- 远距走时表：IASPEI ak135
- SREV 音效（JMA）：[scratch-realtime-earthquake-viewer-page](https://github.com/kotoho7/scratch-realtime-earthquake-viewer-page)
- Open EEW CN 音效（非 JMA 预警 / 速报）：[OpenEEWCN/eewcn](https://github.com/OpenEEWCN/eewcn)
- 中文倒计时播报素材：[地牛 Wake Up!](https://eew.earthquake.tw/)
- 界面字体：[霞鹜文楷](https://github.com/lxgw/LxgwWenKai)（SIL OFL 1.1）

---

## 参考软件

- [Open EEW CN](https://github.com/OpenEEWCN/eewcn)
- [GlobalQuake 0.11](https://github.com/xspanger3770/GlobalQuake)
- [Kanameishi（要石）](https://github.com/Lipomoea/kanameishi)
- [RhythmQuake](https://github.com/yuelinyayun-star/flutterRhythmQuake)
- [Scratch-Realtime-Earthquake-Viewer-Page](https://github.com/kotoho7/scratch-realtime-earthquake-viewer-page)

---

## 版权声明

- [RhythmQuake](https://github.com/yuelinyayun-star/flutterRhythmQuake)
- [Kanameishi](https://github.com/Lipomoea/kanameishi)
- [OPEN EEWCN](https://github.com/OpenEEWCN/eewcn)
- [GlobalQuake 0.11](https://github.com/OpenEEWCN/eewcn)

第三方素材（音效、走时表、底图矢量、断层、字体、地图标记等）不随代码许可证覆盖，请遵循各自上游许可；SREV 音效为 CC BY-SA 2.0，须署名。震度色参考气象厅配色；中国烈度色按 GB/T 相关制图规范摘录。

---

## 特别鸣谢

- [平平无奇的4](https://space.bilibili.com/3707038962027155)
- [悦琳雅韵☆Rhythm(Stille)](https://github.com/yuelinyayun-star)
- [lxfly2000](https://github.com/lxfly2000)
- 以及各位给予帮助的 EEW 爱好者

--- 

## 注意事项

1. **非官方渠道**：本软件仅供信息展示与学习研究，**不构成**各国气象 / 地震主管部门的官方预警渠道。紧急情况下请以主管部门发布为准，并遵从当地防灾指引。
2. **数据免责**：开发者不保证上游数据的实时性、完整性与准确性；网络波动、上游变更或连接超限均可能导致延迟、缺报或显示异常。
3. **令牌与隐私**：Jian 访问令牌（`lk_` / `rt_` / `at_`）请仅保存在本机设置或环境变量中，**勿提交进公开仓库**。
4. **素材授权**：部分音效（如「地牛 Wake Up!」倒计时语音）为非自由许可，默认不随仓库分发；公开再分发前请自行确认授权或更换素材。
5. **连接额度**：同一 IP / 令牌并发有限。若同时开启多个客户端或调试页，可能触发上游连接限制；请关闭多余连接后再试。

## 版权声明

本仓库代码以 [Apache License 2.0](LICENSE) 发布。
