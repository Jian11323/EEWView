# 行政区 / 底图数据

## 暗色矢量（默认）

`basemap = "auto"` 且主题 `dark` 时加载矢量世界轮廓 + 中日区划。

### 世界轮廓优先级

| 顺序 | 路径 | 说明 |
|------|------|------|
| 1 | `globalquake/countries{MD,HD,UHD}.json` | **GlobalQuake Special** 藏南修复版国界（远/中/近分档） |
| 2 | `medium.global.modified.geo.json` | 精细全球国界（展开 GeoJSON） |
| 3 | `kanameishi/medium.global.modified.topo.json` | 要石 TopoJSON 同套边界 |
| 4 | `Globalmap.geo.json` + `world/` | 旧全球轮廓回退 |

GQ Special 世界层已含调整后的中国国界（含藏南）；另叠内置钓鱼岛点状。

### 区划层优先级

| 顺序 | 路径 | 内容 |
|------|------|------|
| 1 | `<省市区>/*.json`、根目录如 `台湾省.json` | **中国地市/区县**（烈度填色与近距描边） |
| 2 | `globalquake/jp-prefectures.geojson` → `日本/` → 要石 | 日本都道府县 |
| 3 | `globalquake/cn-prefectures.geojson` / 要石省级 | 仅当中国扫盘为空时回退；远距外轮廓溶解仍优先用一级区划 |

近距区划与烈度区划填色使用地市/区县边界；烈度按耳切三角形质心估档（大区内随距离衰减，非整块一色）。
有 JMA 官方 `warnArea` 时优先按 `eew/jp.eew.topo.json` 区划名填色，再回退市县三角 / 同心环。详见 [`eew/NOTICE.md`](eew/NOTICE.md)。

要石来源：[Lipomoea/kanameishi](https://github.com/Lipomoea/kanameishi)（AGPL-3.0）。详见 [`kanameishi/README.md`](kanameishi/README.md)。  
GQ Special 说明见 [`globalquake/README.md`](globalquake/README.md)。

## 亮色：自有海洋地形瓦片

主题 `light` 时使用 XYZ：`{tile_base}/{tile_source}/{z}/{x}/{y}`  
默认 `https://tilemap.sismotide.top/arcwob/...`（支持横向循环铺贴）。

暗色 / 球体：近黑底 + 国界/区划轮廓线（不填陆地）。数据优先 `globalquake/`（藏南修复）。  
`globe_dark.png` / `tools/gen_globe_texture.py` 为备用贴图资源，当前运行时不铺贴。

## 活动断层

| 路径 | 内容 |
|------|------|
| `faults/global_active_faults.geojson` | 全球 GEM 活动断层（简化） |
| `faults/china_active_faults.geojson` | 中国及邻区 CAFD / CN-faults（简化） |

平面与球体共用；`ui.show_faults` 开关。详见 [`faults/README.md`](faults/README.md)。

## 其它回退

| 路径 | 内容 |
|------|------|
| `world/cn_boundary_patches.geojson` | 历史补丁（运行时不加载；钓鱼岛已内置） |
