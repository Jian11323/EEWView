# 活动断层数据

运行时加载：

| 文件 | 内容 |
|------|------|
| `global_active_faults.geojson` | 全球活动断层（由 GEM GAF-DB 简化，近距 fine） |
| `global_active_faults_lod0.geojson` | 全球长断层（远距） |
| `global_active_faults_lod1.geojson` | 全球中等断层（中距） |
| `china_active_faults.geojson` | 中国及邻区活断层（由 CAFD2023 / CN-faults 简化，近距 fine） |
| `china_active_faults_lod0.geojson` | 中国长断层（远距） |
| `china_active_faults_lod1.geojson` | 中国中等断层（中距） |

平面图与球体图共用；设置项 `ui.show_faults`。缺 lod 文件时运行时从 fine 按对角线长度现场分档。

## 来源与许可

| 层 | 上游 | 许可 / 引用 |
|----|------|-------------|
| 全球 | [GEM Global Active Faults](https://github.com/GEMScienceTools/gem-global-active-faults) | **CC BY-SA 4.0**；Styron & Pagani (2020), *Earthquake Spectra* |
| 中国 | [gmt-china/china-geospatial-data](https://github.com/gmt-china/china-geospatial-data) `CN-faults` ← CAFD2023 | 徐锡伟团队 CAFD；Wu et al. (2024), *ESSD*；[DOI](https://doi.org/10.5194/essd-16-3391-2024) |

## 重新生成

```bash
python tools/prep_faults.py
```

仅从已有 fine 生成 lod0/lod1（不下载上游）：

```bash
python tools/prep_faults.py --lod-only
```

脚本会下载上游原始数据、简化几何并写出上述 GeoJSON（不把原始大文件提交进仓库）。
