# 行政区 / 底图数据

## 暗色：要石矢量（默认）

`basemap = "auto"` 且主题 `dark` 时，优先加载 `kanameishi/`：

| 路径 | 内容 |
|------|------|
| `kanameishi/medium.global.modified.topo.json` | 世界轮廓 |
| `kanameishi/cn.province.topo.json` | 中国省级 |
| `kanameishi/jp.pref.topo.json` | 日本都道府县 |

来源：[Lipomoea/kanameishi](https://github.com/Lipomoea/kanameishi)（AGPL-3.0）。详见 [`kanameishi/README.md`](kanameishi/README.md)。

## 亮色：自有海洋地形瓦片

主题 `light` 时使用 XYZ：`{tile_base}/{tile_source}/{z}/{x}/{y}`  
默认 `https://tilemap.sismotide.top/arcwob/...`（支持横向循环铺贴）。

## 回退（无要石数据时）

| 路径 | 内容 |
|------|------|
| `Globalmap.geo.json` | 全球国家轮廓 |
| `world/cn_boundary_patches.geojson` | 藏南、钓鱼岛等补丁 |
| `*/`、`日本/` | 中国地市 / 日本都道府县 |
