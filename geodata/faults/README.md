# 活动断层数据

运行时加载：

| 文件 | 内容 |
|------|------|
| `global_active_faults.geojson` | 全球活动断层（由 GEM GAF-DB 简化） |
| `china_active_faults.geojson` | 中国及邻区活断层（由 CAFD2023 / CN-faults 简化） |

平面图与球体图共用；设置项 `ui.show_faults`。

## 来源与许可

| 层 | 上游 | 许可 / 引用 |
|----|------|-------------|
| 全球 | [GEM Global Active Faults](https://github.com/GEMScienceTools/gem-global-active-faults) | **CC BY-SA 4.0**；Styron & Pagani (2020), *Earthquake Spectra* |
| 中国 | [gmt-china/china-geospatial-data](https://github.com/gmt-china/china-geospatial-data) `CN-faults` ← CAFD2023 | 徐锡伟团队 CAFD；Wu et al. (2024), *ESSD*；[DOI](https://doi.org/10.5194/essd-16-3391-2024) |

## 重新生成

```bash
python tools/prep_faults.py
```

脚本会下载上游原始数据、简化几何并写出上述两个 GeoJSON（不把原始大文件提交进仓库）。
