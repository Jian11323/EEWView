# EEWView · 地震视监器

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

纯 Rust 地震预警 / 速报桌面客户端。  

---

## 地震视监器

| 区域 | 内容 |
|------|------|
| 顶栏健康 | `Jian` · `Wolfx` · `P2P` → **正常 / 波动 / 异常** |
| Records | 速报历史（地名 / 时间 / M / 震度色块） |
| EEW | 预警列表（报数 / M / 最大震度） |
| Station | 测站列表 |
| 地图 | 底图瓦片 + 震中 + P/S 波圈（JMA2001 走时） |
| 信息头 / 倒计时 | 当前事件；本机经纬有效时显示 S 波剩余秒 |

点击列表项可将地图居中到对应事件或测站，并刷新信息头。

---

## 数据源

| 源 | 用途 | 配置 |
|----|------|------|
| **Jian Project API** | 预警 / 速报 / 测站 / 底图 | `[sources.jian]`；访问令牌用环境变量，勿写入仓库 |
| **Wolfx** | JMA / CENC 等预警与列表补充 | `[sources.wolfx]` |
| **P2PQuake** | 气象厅地震情报等 | `[sources.p2pquake]` |

底图：`https://tilemap.sismotide.top/{source}/{z}/{x}/{y}`（默认 `arcwob`）。  
字段约定见 [`docs/数据源.md`](docs/数据源.md)。

---

## 音效

| 用途 | 素材 | 说明 |
|------|------|------|
| 预警事件 | [SREV](assets/sound/ATTRIBUTION.md) | CC BY-SA 2.0，须署名 |
| 本地 S 波倒计时 | 地牛 Wake Up! | 非自由许可；发行前须授权或替换；默认不随仓库分发 |
| 测站 / UI | 原创合成 | CC0 |

路由表：`assets/sound/catalog.json`；说明见 [`docs/音效路由.md`](docs/音效路由.md)。

---

## 走时与色标

| 项 | 说明 |
|----|------|
| 近距走时 | 气象厅 **JMA2001**（[`ATTRIBUTION`](assets/travel/ATTRIBUTION.md)） |
| 远距走时 | IASPEI **ak135**（回退表） |
| 震度色 | 气象厅配色指针表２－２ |
| 烈度色 | 按 GB/T 38226 制图规范摘录（见 `assets/palette/`） |

---

## 构建运行

需要：Rust（edition 2021）；Windows 建议安装 VS Build Tools（编译 eframe）。

```bash
# 若尚无走时 JSON
cargo run -p convert_travel_tables

# 启动
cargo run -p jian-app
```

本机位置（倒计时与本地 S 波 ETA）：

```toml
# config/default.toml 或本机覆盖配置
[local]
latitude = 35.68
longitude = 139.76
```

Jian 访问令牌（PowerShell 示例）：

```bash
$env:JIAN_ACCESS_TOKEN = "at_…"
cargo run -p jian-app
```

未配置令牌时仍可使用 Wolfx 与 P2PQuake；Jian 健康态为「异常」。

---

## 目录

| 路径 | 说明 |
|------|------|
| `crates/` | 应用、界面、地图、网络、音效、走时等 |
| `assets/` | 音效、走时表、色标、图标 |
| `config/default.toml` | 默认配置（密钥勿提交） |
| `docs/` | 架构与接口说明 |
| `tools/` | 走时表转换等工具 |

架构总览：[`docs/项目结构规划.md`](docs/项目结构规划.md)。

---

## License

**代码**：[Apache License 2.0](LICENSE)

第三方素材不随代码许可证覆盖：

- [`assets/sound/ATTRIBUTION.md`](assets/sound/ATTRIBUTION.md)
- [`assets/travel/ATTRIBUTION.md`](assets/travel/ATTRIBUTION.md)
