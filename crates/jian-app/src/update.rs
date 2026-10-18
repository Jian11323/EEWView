//! 从 GitHub Releases 检查 / 下载 / 应用更新。

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

const RELEASES_API: &str =
    "https://api.github.com/repos/Jian11323/StarQuake/releases/latest";
const UA: &str = concat!(
    "StarQuake/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Jian11323/StarQuake)"
);

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub asset_url: String,
    pub asset_name: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// 解析 `v2.0.1` / `2.0.1` / `StarQuake-2.0.1` → (major, minor, patch)。
pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let s = raw.trim().trim_start_matches('v').trim_start_matches('V');
    let s = s
        .rsplit([' ', '-', '_'])
        .find(|p| p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or(s);
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(remote: &str, local: &str) -> bool {
    match (parse_semver(remote), parse_semver(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn pick_asset(assets: &[GhAsset]) -> Option<&GhAsset> {
    #[cfg(windows)]
    {
        assets
            .iter()
            .find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.contains("portable") && n.ends_with(".zip") && n.contains("starquake")
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.ends_with(".zip") && n.contains("starquake")
                })
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    (n.contains("setup") || n.contains("installer")) && n.ends_with(".exe")
                })
            })
    }
    #[cfg(not(windows))]
    {
        assets
            .iter()
            .find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.contains("linux") && (n.ends_with(".tar.gz") || n.ends_with(".tgz"))
            })
            .or_else(|| {
                assets.iter().find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.contains("starquake") && n.ends_with(".tar.gz")
                })
            })
    }
}

/// 查询最新版；若无更新或失败返回 `Ok(None)` / `Err`。
pub fn check_latest(local_version: &str) -> Result<Option<AvailableUpdate>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let resp = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("请求 GitHub Releases")?;
    if !resp.status().is_success() {
        bail!("GitHub Releases HTTP {}", resp.status());
    }
    let rel: GhRelease = resp.json().context("解析 Releases JSON")?;
    let ver = rel.tag_name.trim().trim_start_matches('v').to_string();
    if !is_newer(&ver, local_version) {
        info!(local = local_version, remote = %ver, "已是最新版");
        return Ok(None);
    }
    let Some(asset) = pick_asset(&rel.assets) else {
        bail!("最新 Release 无匹配本平台的安装包资源");
    };
    Ok(Some(AvailableUpdate {
        version: ver,
        name: rel.name.unwrap_or_default(),
        body: rel.body.unwrap_or_default(),
        html_url: rel.html_url,
        asset_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
    }))
}

fn staging_root() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("StarQuake_update");
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 下载资源到暂存目录，返回本地文件路径。
pub fn download_asset(info: &AvailableUpdate) -> Result<PathBuf> {
    let dir = staging_root()?;
    let dest = dir.join(&info.asset_name);
    info!(url = %info.asset_url, dest = %dest.display(), "下载更新包");
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let mut resp = client
        .get(&info.asset_url)
        .send()
        .context("下载更新包")?;
    if !resp.status().is_success() {
        bail!("下载失败 HTTP {}", resp.status());
    }
    let mut file = File::create(&dest)?;
    io::copy(&mut resp, &mut file)?;
    file.flush()?;
    Ok(dest)
}

/// 应用已下载的更新包并重启（Windows 便携 zip / setup.exe；Linux tar.gz）。
pub fn apply_and_relaunch(package: &Path) -> Result<()> {
    let exe = std::env::current_exe().context("current_exe")?;
    let app_dir = exe
        .parent()
        .map(Path::to_path_buf)
        .context("exe parent")?;
    let name = package
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if name.ends_with(".exe") {
        Command::new(package).spawn().context("启动安装包")?;
        std::process::exit(0);
    }

    let extract_dir = package
        .parent()
        .unwrap_or(Path::new("."))
        .join("extracted");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)?;

    if name.ends_with(".zip") {
        extract_zip(package, &extract_dir)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(package, &extract_dir)?;
    } else {
        bail!("不支持的更新包格式: {name}");
    }

    let payload = find_payload_root(&extract_dir)?;
    #[cfg(windows)]
    {
        let bat = staging_root()?.join("apply_update.cmd");
        let exe_name = exe
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("starquake.exe");
        let body = format!(
            "@echo off\r\n\
chcp 65001 >nul\r\n\
timeout /t 2 /nobreak >nul\r\n\
xcopy /E /Y /I /Q \"{src}\\*\" \"{dst}\\\"\r\n\
start \"\" \"{dst}\\{exe}\"\r\n\
del \"%~f0\"\r\n",
            src = payload.display(),
            dst = app_dir.display(),
            exe = exe_name,
        );
        fs::write(&bat, body)?;
        Command::new("cmd")
            .args(["/C", &bat.to_string_lossy()])
            .spawn()
            .context("启动更新脚本")?;
        std::process::exit(0);
    }
    #[cfg(not(windows))]
    {
        copy_dir_recursive(&payload, &app_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = app_dir.join("starquake");
            if bin.is_file() {
                let mut perms = fs::metadata(&bin)?.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&bin, perms);
            }
        }
        Command::new(&exe).spawn().context("重启应用")?;
        std::process::exit(0);
    }
}

fn find_payload_root(extract_dir: &Path) -> Result<PathBuf> {
    // 打包顶层多为 StarQuake/ 或 EEWView/
    for name in ["StarQuake", "EEWView", "starquake", "eewview"] {
        let p = extract_dir.join(name);
        if p.is_dir() {
            return Ok(p);
        }
    }
    let mut dirs = fs::read_dir(extract_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    if dirs.len() == 1 {
        return Ok(dirs.remove(0));
    }
    Ok(extract_dir.to_path_buf())
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).context("打开 zip")?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out = match entry.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if entry.name().ends_with('/') {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut outfile = File::create(&out)?;
        io::copy(&mut entry, &mut outfile)?;
    }
    Ok(())
}

fn extract_tar_gz(path: &Path, dest: &Path) -> Result<()> {
    // 无额外依赖：调用系统 tar
    let status = Command::new("tar")
        .args(["-xzf", &path.to_string_lossy(), "-C", &dest.to_string_lossy()])
        .status()
        .context("执行 tar")?;
    if !status.success() {
        bail!("tar 解压失败");
    }
    Ok(())
}

#[cfg(not(windows))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for ent in fs::read_dir(src)? {
        let ent = ent?;
        let from = ent.path();
        let to = dst.join(ent.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn open_release_page(url: &str) {
    #[cfg(windows)]
    {
        if let Err(e) = Command::new("cmd").args(["/C", "start", "", url]).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Err(e) = Command::new("open").arg(url).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Err(e) = Command::new("xdg-open").arg(url).spawn() {
            warn!("无法打开发布页: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_newer() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("v2.1.0", "2.0.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }
}
