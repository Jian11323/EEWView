# 行政区 / 底图数据

## 暗色矢量（默认）

`basemap = "auto"` 且主题 `dark` 时加载矢量世界轮廓 + 中日**省级**区划。

平面与球体按当前投影**分别加载**，切换投影时重载，避免两套几何同时常驻。

### 世界轮廓优先级

| 顺序 | 路径 | 说明 |
|------|------|------|
| 1 | `globalquake/countriesMD.json` | GlobalQuake Special 藏南修复国界（单档） |
| 2 | `kanameishi/medium.global.modified.topo.json` | 要石 TopoJSON 回退 |
| 3 | `world/` | 粗全球轮廓回退 |

另叠内置钓鱼岛点状。不再加载 HD/UHD、地市/区县或美新区划细图。

### 区划层

| 路径 | 内容 |
|------|------|
| `globalquake/cn-prefectures.geojson` | 中国省级（含藏南） |
| `globalquake/jp-prefectures.geojson` | 日本都道府县 |
| 要石 `cn_province` / `jp_pref` | 上述缺失时回退 |

## 活动断层

`ui.show_faults` 默认关闭；**仅开启后才加载与绘制**。详见 [`faults/README.md`](faults/README.md)。
