# EEW View · 地震视监器

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

纯 Rust 地震预警 / 速报桌面客·端。  

---

## 地震视监器

| 区域 | 内容 |
|------|------|
| 健·点 | `Jian` · `Wolfx` · `P2P` → **正常 / 波动 / 异常** |
| 速报 | 速报历史（地名 / 时间 / M / 震度色块） |
| 预警 | 预警列表（报数 / M / 最大震度） |
| 测站 | NIED Kmoni 有感测站（列表 + 地图色点） |
| 地图 | 暗色要石矢量 / 亮色地形瓦片 + 震中 + P/S 波圈（JMA2001 走时） |
| 图例 / 时钟 | ·下烈度色标；右下本地时间（可加 JST） |
| 信息头 / 倒计时 | 当前事件；本机经纬有效时 S 波剩余秒叠在右下时钟上方 |
| 主题 | 设置 → 显示：暗色 / 亮色（主界面色板 + auto 底图同步切换） |
| 设置 | API与数据源 / 地图与显示 / 音频 / 性能与渲染 / 高级 / 关于（主窗内嵌设置） |

点击列表项可将地图居中到对应事件或测站，并·新信息头。

---

## 数据源

| 源 | 用途 | 配置 |
|----|------|------|
| **Jian Project API** | 预警 / 速报 / 测站 / 底图 | `[sources.jian]`；设置页用 `lk_` 换 `rt_`，或环境变量 `JIAN_ACCESS_TOKEN` / `JIAN_REFRESH_TOKEN` |
| **Wolfx** | JMA / CENC 等预警与列表补充 | `[sources.wolfx]` |
| **P2PQuake** | 气象厅地震情报等 | `[sources.p2pquake]` |

底图：`basemap = "auto"` 时暗色用 `geodata/kanameishi/` 要石矢量，亮色用自有·洋地形瓦片 `tilemap.sismotide.top/arcwob`（横向循环）。  
国外源地名按融合历史 FE 格网修正（`assets/place/`）；JMA / CWA / HKO / CENC 保留原文。  
字段约定见 [`docs/数据源.md`](docs/数据源.md)；底图说明见 [`geodata/README.md`](geodata/README.md)。

---

## 音效

| 用途 | 素材 | 说明 |
|------|------|------|
| 预警事件 | [SREV](assets/sound/ATTRIBUTION.md) | CC BY-SA 2.0，须署名 |
| 本地 S 波倒计时 | 地牛 Wake Up! | 非自由许可；发行前须授权或替换；默认不随仓库分发 |
| 测站 / UI | 原创合成 | CC0 |

·由表：`assets/sound/catalog.json`；说明见 [`docs/音效·由.md`](docs/音效·由.md)。

---

## 走时与色标

| 项 | 说明 |
|----|------|
| 近·走时 | 气象厅 **JMA2001**（[`ATTRIBUTION`](assets/travel/ATTRIBUTION.md)） |
| 远·走时 | IASPEI **ak135**（回退表） |
| 震度色 | 气象厅配色指针表２－２ |
| 烈度色 | 按 GB/T 38226 制图规范摘录（见 `assets/palette/`） |

---

## 版本

当前通道：**Beta**，版本·与 Cargo workspace 一致（现为 **1.1.0**）。界面展示为 `Beta x.y.z`。

| 变更性质 | 递增位 | 示例 |
|----------|--------|------|
| 缺·修复、文案/·式微调、性能小修、无对外行为变化 | **PATCH**（第三位） | 1.0.0 → 1.0.1 |
| 新功能、新数据源/设置项、交互或展示行为有意变更 | **MINOR**（第二位） | 1.0.1 → 1.1.0 |
| 配置/协议不兼容，或结束 Beta 转正式版 | **MAJOR**（第一位） | 1.x → 2.0.0；正式版可去掉 Beta 标 |

改版本时同步：`Cargo.toml` → `[workspace.package].version`，并重新打包便携包。

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

未配置令牌时仍可使用 Wolfx 与 P2PQuake；Jian 健·态为「异常」。

发行打包（产物在 `dist/`，不进 Git）：

```bash
# Windows 便携包
python tools/pack_windows.py
# Windows 安装包（NSIS → setup.exe；需先有便携目录或本脚本会自动组装）
python tools/pack_windows_installer.py

# Linux x86_64 便携包（本机 Linux / WSL，或 Windows 交叉 cargo zigbuild 后）
# cargo zigbuild -p jian-app --release --target x86_64-unknown-linux-gnu.2.31
python tools/pack_linux.py --skip-build --bin target/x86_64-unknown-linux-gnu/release/jian

# NixOS / flake（在已装 Nix 的机器上）
# nix run .#
# nix profile install .#
python tools/pack_nix.py
```

资源根默认是可执行文件所在目录（或其上级仓库根）；也可用环境变量 `EEWVIEW_ROOT` 指定。

运行日志默认写到资源根下 `logs/eewview.log.YYYY-MM-DD`（按日滚动）；设置 → 高级 → 运行日志可改等级与打开目录。环境变量 `RUST_LOG` 优先于配置中的等级。

---

## 目录

| ·径 | 说明 |
|------|------|
| `crates/` | 应用、界面、地图、网络、音效、走时等 |
| `assets/` | 音效、走时、色标、烈度图标、地图标记、品牌、霞鹜文· |
| `geodata/` | 暗色要石 TopoJSON；可选行政区 GeoJSON 回退 |
| `config/default.toml` | 默认配置（密钥勿提交） |
| `docs/` | 架构与接口说明 |
| `tools/` | 走时表转换等·· |

架构总览：[`docs/项目结构规划.md`](docs/项目结构规划.md)。

---

## License

**代码**：[Apache License 2.0](LICENSE)

第三方素材不随代码许可证覆盖：

- [`assets/sound/ATTRIBUTION.md`](assets/sound/ATTRIBUTION.md)
- [`assets/travel/ATTRIBUTION.md`](assets/travel/ATTRIBUTION.md)
- [`assets/ATTRIBUTION.md`](assets/ATTRIBUTION.md)
- [`assets/place/ATTRIBUTION.md`](assets/place/ATTRIBUTION.md)
