# 行政区 / 底图数据

## 暗色矢量（默认）

`basemap = "auto"` 且主题 `dark` 时加载矢量世界轮廓 + 中日区划。

### 世界轮廓优先级

| 顺序 | 路径 | 说明 |
|------|------|------|
| 1 | `medium.global.modified.geo.json` | 精细全球国界（展开 GeoJSON） |
| 2 | `kanameishi/medium.global.modified.topo.json` | 要石 TopoJSON 同套边界 |
| 3 | `Globalmap.geo.json` + `world/` | 旧全球轮廓回退 |

世界层本身不含中国 / 日本外轮廓（由区划层绘制）。藏南由 `cn.province` 西藏面叠盖世界层，不另画凸包；钓鱼岛用内置点状。

### 区划层

| 路径 | 内容 |
|------|------|
| `kanameishi/cn.province.topo.json` | 中国省级 |
| `kanameishi/jp.pref.topo.json` | 日本都道府县 |

要石来源：[Lipomoea/kanameishi](https://github.com/Lipomoea/kanameishi)（AGPL-3.0）。详见 [`kanameishi/README.md`](kanameishi/README.md)。

## 亮色：自有海洋地形瓦片

主题 `light` 时使用 XYZ：`{tile_base}/{tile_source}/{z}/{x}/{y}`  
默认 `https://tilemap.sismotide.top/arcwob/...`（支持横向循环铺贴）。

球体投影模式下不贴瓦片，始终使用矢量底图。

## 其它回退

| 路径 | 内容 |
|------|------|
| `world/cn_boundary_patches.geojson` | 历史补丁（运行时不加载；钓鱼岛已内置） |
| `*/`、`日本/` | 中国地市 / 日本都道府县（无要石区划时） |
