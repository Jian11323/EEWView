# 走时表来源

## JMA2001（主表）

| 项 | 内容 |
|----|------|
| 来源 | 気象庁「走時表・射出角表・速度構造データファイル」 |
| 下载 | https://www.data.jma.go.jp/eqev/data/bulletin/catalog/appendix/trtime/trt_j.html |
| 运行时 | `jma2001.json`（由 `tools/convert_travel_tables` 自官方文件转换） |
| 原始包 | 可放 `raw/`，体积大，不进 Git（见 `.gitignore`） |

## ak135（远距 / 全球）

| 项 | 内容 |
|----|------|
| 模型 | Kennett, Engdahl & Buland (1995), Geophys. J. Int. |
| IASPEI | http://download.iaspei.org/download/AK135tables.pdf |
| 速度模型 | https://www.auspass.edu.au/research/ak135/ak135.t.txt |
| 运行时 | `ak135_stub.json`（远距回退用；可用 iaspei-tau 生成表替换） |

关于页须注明气象厅 JMA2001 与 IASPEI ak135 出处。
