# EEWView · 地震视监器

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-WIP%20未完成-orange.svg)](#⚠-项目未完成请勿当作成品使用)

纯 Rust 地震预警 / 速报桌面客户端。  
仓库：<https://github.com/Jian11323/EEWView>

---

## ⚠ 项目未完成，请勿当作成品使用

**本仓库当前为开发中骨架（WIP），不是可依赖的正式客户端。**

- 功能不完整：P0–P4 仅为骨架；测站完善、设置页、发行打包未做  
- 行为不稳定：可能崩溃、鉴权不全、配置与文档会随时改  
- **不建议**下载后直接用于实际避险或对外二次分发「成品包」  
- 欢迎关注进度；需要稳定产品请等待后续正式版本 / Release 说明  

完成后会在本 README 与 Release 中明确标注「可用」；在此之前请以 **未完成** 为准。


## 界面（地震情报实况栏）

右侧实况栏与主视口对齐 [eewcn](https://github.com/OpenEEWCN/eewcn) 分区（仅布局参考，自写 egui，不移植 QML）：

| 区域 | 内容 |
|------|------|
| 顶栏健康 | `Jian` · `Wolfx` · `P2P` → **正常 / 波动 / 异常** |
| Records | 速报历史（地名 / 时间 / M / 震度色块） |
| EEW | 预警列表（报数 / M / 最大震度） |
| Station | 测站列表 |
| 地图 | 底图瓦片 + 震中 + P/S 波圈（JMA2001 走时） |
| 信息头 / 倒计时 | 当前事件；本机经纬有效时显示 S 波剩余秒 |

## 数据源

| 源 | 用途 | 配置 |
|----|------|------|
| **Jian Project API** | 自建预警 / 测站 / 底图 | `config/default.toml` → `[sources.jian]`；Token 用环境变量或本机配置，勿提交 |
| **Wolfx** | JMA / CENC 等 EEW 补充 | `[sources.wolfx]` |
| **P2PQuake** | 气象厅地震情报等 | `[sources.p2pquake]` |

底图默认：`https://tilemap.sismotide.top/arcwob/{z}/{x}/{y}`。

## 进度

| 阶段 | 状态 |
|------|------|
| P0 主界面分区 + 官方色标 | 骨架完成（仍属 WIP） |
| P1 瓦片底图 + 走时 P/S 圈 | 骨架完成（仍属 WIP） |
| P2 列表 → 地图 / Header 联动 | 骨架完成（仍属 WIP） |
| P3 三源网络 | 骨架可跑（仍属 WIP，鉴权/测站未齐） |
| P4 音效落地 | 骨架可跑（SREV 事件 + 倒计时状态机；地牛文件本机自备） |
| P5 测站完善 | **下一步 / 未完成** |
| P6 设置 / 打包 / 正式版 | 未开始 |

## 构建运行（仅开发自测）

需要：Rust（edition 2021）；Windows 下建议安装 VS Build Tools（编译 eframe）。

```bash
# 首次：原始走时表 → JSON（若尚无 assets/travel/jma2001.json）
cargo run -p convert_travel_tables

# 启动（开发用，勿当成品）
cargo run -p jian-app
```

可选：在 `config/default.toml` 的 `[local]` 填写本机经纬度，倒计时按走时表计算并驱动地牛播报。

```toml
[local]
latitude = 35.68
longitude = 139.76
```

Jian 源需访问令牌（勿提交仓库）：

```bash
# PowerShell
$env:JIAN_ACCESS_TOKEN = "at_…"
cargo run -p jian-app
```

无 Token 时仍会连 Wolfx / P2PQuake；Jian 健康态显示「异常」。音效细节见 [`docs/音效路由.md`](docs/音效路由.md)，数据源见 [`docs/数据源.md`](docs/数据源.md)。

## 目录

| 路径 | 说明 |
|------|------|
| `crates/` | 业务 crate（app / ui / map / net / travel / …） |
| `assets/` | 音效、走时表、色标、图标 |
| `config/default.toml` | 默认配置（密钥勿提交；可用 `config/local.toml`） |
| `docs/` | 设计与约定 |
| `tools/` | 走时转换等工具 |

## 音效与色标

- 默认事件音：[SREV](assets/sound/ATTRIBUTION.md)（CC BY-SA 2.0，须署名）
- 倒计时：地牛 Wake Up! 素材（**非自由许可**，公开再分发前须授权或替换）
- 震度色：气象厅配色指针；烈度色：待 GB/T 38226 正式摘录
- 走时：气象厅 [JMA2001](assets/travel/ATTRIBUTION.md)；远距 ak135（当前 stub）

## License

**代码**：[Apache License 2.0](LICENSE)

第三方素材不随代码许可证覆盖，见：

- [`assets/sound/ATTRIBUTION.md`](assets/sound/ATTRIBUTION.md)
- [`assets/travel/ATTRIBUTION.md`](assets/travel/ATTRIBUTION.md)

更完整的架构说明：[`docs/项目结构规划.md`](docs/项目结构规划.md)。
