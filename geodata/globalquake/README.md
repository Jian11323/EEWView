# GlobalQuake Special 矢量底图

数据来自本地 **GlobalQuake Special**：

- 国界：`GlobalQuakeCore/src/main/resources/polygons/`（`countriesMD/HD/UHD.json`，含藏南等争议地区修正）
- 区划：`.../polygons_converted/`（中国一级行政区、日本都道府县、美/阿/新/夏威夷/意大利等）

运行时由 `jian-map` 按 GQ `GlobePanel` 的 scroll 分档绘制：

| scroll | 图层 |
|--------|------|
| ≥0.5 | `countriesMD` |
| 0.25–0.5 | `countriesHD`（滤除中/日/美/新）+ 中日美新区划填色 |
| <0.25 | `countriesUHD`（同上过滤）+ 区划；意大利 `<0.20` |

| 文件 | 用途 |
|------|------|
| `countriesMD.json` | 远距世界国界 |
| `countriesHD.json` | 中距世界国界 |
| `countriesUHD.json` | 近距世界国界 |
| `cn-prefectures.geojson` | 中国一级行政区（含藏南；远距轮廓/回退；近距烈度优先用上级目录省市区县） |
| `jp-prefectures.geojson` | 日本都道府县 |
| `us-albers.geojson` 等 | 美/阿/新/夏威夷/意大利区划 |

许可：原 GlobalQuake 为 MIT；Special 仓修改数据按上游说明使用。
