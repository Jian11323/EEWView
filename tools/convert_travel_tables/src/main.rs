//! 将气象厅 tjma2001 原始文件转为 jma2001.json。
//!
//! 用法（仓库根）:
//!   cargo run -p convert_travel_tables

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct OutTable {
    name: String,
    source: String,
    depths_km: Vec<f64>,
    distances_km: Vec<f64>,
    p_times_s: Vec<Vec<f64>>,
    s_times_s: Vec<Vec<f64>>,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn parse_jma2001(path: &Path) -> Result<OutTable> {
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    // depth -> (dist -> (p,s))
    let mut by_depth: BTreeMap<i32, BTreeMap<i32, (f64, f64)>> = BTreeMap::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // 格式: P f8.3 S f8.3 I3 depth I5 distance  （固定宽，也可用 split）
        // 例: "P    0.416 S    0.703   0      2"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            bail!("line {}: bad columns: {line}", lineno + 1);
        }
        // P <p> S <s> <depth> <dist>
        if parts[0] != "P" || parts[2] != "S" {
            bail!("line {}: expected P/S markers", lineno + 1);
        }
        let p: f64 = parts[1].parse()?;
        let s: f64 = parts[3].parse()?;
        let depth: i32 = parts[4].parse()?;
        let dist: i32 = parts[5].parse()?;
        by_depth.entry(depth).or_default().insert(dist, (p, s));
    }

    let depths: Vec<i32> = by_depth.keys().copied().collect();
    let distances: Vec<i32> = by_depth
        .values()
        .next()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();

    let mut p_times = Vec::with_capacity(depths.len());
    let mut s_times = Vec::with_capacity(depths.len());
    for &d in &depths {
        let row = by_depth.get(&d).unwrap();
        let mut pr = Vec::with_capacity(distances.len());
        let mut sr = Vec::with_capacity(distances.len());
        for &x in &distances {
            let (p, s) = row.get(&x).copied().unwrap_or((f64::NAN, f64::NAN));
            pr.push(p);
            sr.push(s);
        }
        p_times.push(pr);
        s_times.push(sr);
    }

    Ok(OutTable {
        name: "jma2001".into(),
        source: "JMA tjma2001 — https://www.data.jma.go.jp/eqev/data/bulletin/catalog/appendix/trtime/trt_j.html".into(),
        depths_km: depths.into_iter().map(|v| v as f64).collect(),
        distances_km: distances.into_iter().map(|v| v as f64).collect(),
        p_times_s: p_times,
        s_times_s: s_times,
    })
}

/// ak135 远距回退表：浅层常速网格；可用 iaspei-tau 生成表替换。
fn write_ak135_stub(out: &Path) -> Result<()> {
    let depths: Vec<f64> = (0..=700).step_by(50).map(|v| v as f64).collect();
    let distances: Vec<f64> = (0..=10000).step_by(100).map(|v| v as f64).collect();
    let mut p_times = Vec::new();
    let mut s_times = Vec::new();
    for &dep in &depths {
        let mut pr = Vec::new();
        let mut sr = Vec::new();
        for &dist in &distances {
            let hyp = (dep * dep + dist * dist).sqrt();
            pr.push(hyp / 8.0); // 粗略远距 P
            sr.push(hyp / 4.5); // 粗略远距 S
        }
        p_times.push(pr);
        s_times.push(sr);
    }
    let table = OutTable {
        name: "ak135_stub".into(),
        source: "IASPEI ak135 fallback (Kennett et al.); replaceable via iaspei-tau".into(),
        depths_km: depths,
        distances_km: distances,
        p_times_s: p_times,
        s_times_s: s_times,
    };
    let json = serde_json::to_string(&table)?;
    fs::write(out, json)?;
    Ok(())
}

fn main() -> Result<()> {
    let root = workspace_root();
    let raw = root.join("assets/travel/raw/tjma2001/tjma2001");
    let out_dir = root.join("assets/travel");
    fs::create_dir_all(&out_dir)?;

    println!("parsing {} ...", raw.display());
    let table = parse_jma2001(&raw)?;
    println!(
        "  depths={} distances={} cells={}",
        table.depths_km.len(),
        table.distances_km.len(),
        table.depths_km.len() * table.distances_km.len()
    );
    let out = out_dir.join("jma2001.json");
    fs::write(&out, serde_json::to_string(&table)?)?;
    println!("wrote {}", out.display());

    let stub = out_dir.join("ak135_stub.json");
    write_ak135_stub(&stub)?;
    println!("wrote {}", stub.display());
    Ok(())
}
