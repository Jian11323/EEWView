# 走时表来源

## JMA2001（主表）

| 项 | 内容 |
|----|------|
| 来源 | 気象庁「走時表・射出角表・速度構造データファイル」 |
| 下载 | https://www.data.jma.go.jp/eqev/data/bulletin/catalog/appendix/trtime/trt_j.html |
| 原始 | `raw/tjma2001.zip` → `raw/tjma2001/tjma2001` |
| 运行时 | `jma2001.json`（由 `tools/convert_travel_tables` 转换） |
| 另存 | `raw/tjma2001h.zip`（JMA2001A 陆域）、`raw/vjma2001.zip`（速度结构） |

## ak135（远距 / 全球）

| 项 | 内容 |
|----|------|
| 模型 | Kennett, Engdahl & Buland (1995), Geophys. J. Int. |
| IASPEI 表册 | `raw/ak135tables.pdf` — http://download.iaspei.org/download/AK135tables.pdf |
| 速度模型 | `raw/ak135.tvel` — https://www.auspass.edu.au/research/ak135/ak135t.txt |
| 运行时 | 当前 `ak135_stub.json` 为**占位粗表**；正式应以 iaspei-tau 生成后替换 |

关于页须注明气象厅 JMA2001 与 IASPEI ak135 出处。
