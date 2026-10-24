//! 从静态更新清单检查 / 下载 / 应用更新。
//!
//! 并行拉取 GitHub 与 Gitee 上的 `update/manifest.json` 与更新包，先成功者优先。

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use tracing::{info, warn};

const UA: &str = concat!(
    "StarQuake/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Jian11323/StarQuake; +https://gitee.com/jian0786/starquake)"
);

/// 清单地址（GitHub / Gitee 并行竞速，先成功者优先）。
const MANIFEST_URLS: &[&str] = &[
    "https://gitee.com/jian0786/starquake/raw/master/update/manifest.json",
    "https://gitee.com/jian0786/starquake/raw/main/update/manifest.json",
    "https://raw.githubusercontent.com/Jian11323/StarQuake/main/update/manifest.json",
    "https://raw.githubusercontent.com/Jian11323/StarQuake/master/update/manifest.json",
];

const GITEE_REPO_PREFIX: &str = "https://gitee.com/jian0786/starquake/";
const GITHUB_REPO_PREFIX: &str = "https://github.com/Jian11323/StarQuake/";

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub asset_url: String,
    pub asset_name: String,
    /// 空则跳过校验
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    latest_version: String,
    #[serde(default)]
    min_supported_version: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    #[cfg_attr(not(windows), allow(dead_code))]
    installer: Option<ManifestAsset>,
    #[serde(default)]
    #[cfg_attr(not(windows), allow(dead_code))]
    zip: Option<ManifestAsset>,
    #[serde(default)]
    #[cfg_attr(windows, allow(dead_code))]
    linux: Option<ManifestAsset>,
    /// 更新日志正文（弹窗展示）
    #[serde(default)]
    notes: String,
    #[serde(default)]
    release_notes_url: String,
    #[serde(default)]
    html_url: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ManifestAsset {
    url: String,
    #[serde(default)]
    sha256: String,
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

fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?)
}

fn try_fetch_manifest_one(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<UpdateManifest> {
    let resp = client.get(url).send().with_context(|| format!("请求 {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        let snippet: String = body.chars().take(160).collect();
        bail!("HTTP {status} {snippet}");
    }
    // 兼容带 BOM 的 UTF-8
    let bytes = resp.bytes().context("读取清单正文")?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str::<UpdateManifest>(text).context("解析 JSON")
}

fn fetch_manifest(client: &reqwest::blocking::Client) -> Result<UpdateManifest> {
    let (tx, rx) = mpsc::channel();
    let n = MANIFEST_URLS.len();
    for url in MANIFEST_URLS {
        let tx = tx.clone();
        let client = client.clone();
        let url = (*url).to_string();
        std::thread::spawn(move || {
            let result = try_fetch_manifest_one(&client, &url);
            let _ = tx.send((url, result));
        });
    }
    drop(tx);

    let mut errors = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        match rx.recv() {
            Ok((url, Ok(m))) => {
                info!(url = %url, ver = %m.latest_version, "更新清单已加载");
                return Ok(m);
            }
            Ok((url, Err(e))) => {
                errors.push(format!("{url} → {e:#}"));
                remaining -= 1;
            }
            Err(_) => break,
        }
    }
    bail!(
        "无法获取更新清单：{}",
        if errors.is_empty() {
            "无可用镜像".into()
        } else {
            errors.join("；")
        }
    )
}

fn pick_platform_asset(m: &UpdateManifest) -> Result<(ManifestAsset, String)> {
    #[cfg(windows)]
    {
        // 便携 zip 优先，其次 setup.exe（对齐旧版 GitHub 资源挑选）
        if let Some(z) = m.zip.as_ref().filter(|a| !a.url.trim().is_empty()) {
            let name = url_file_name(&z.url).unwrap_or_else(|| "StarQuake_portable.zip".into());
            return Ok((z.clone(), name));
        }
        if let Some(i) = m.installer.as_ref().filter(|a| !a.url.trim().is_empty()) {
            let name = url_file_name(&i.url).unwrap_or_else(|| "StarQuake_setup.exe".into());
            return Ok((i.clone(), name));
        }
        bail!("更新清单缺少 Windows 安装包 / 便携包 URL");
    }
    #[cfg(not(windows))]
    {
        if let Some(l) = m.linux.as_ref().filter(|a| !a.url.trim().is_empty()) {
            let name = url_file_name(&l.url).unwrap_or_else(|| "StarQuake_linux.tar.gz".into());
            return Ok((l.clone(), name));
        }
        bail!("更新清单缺少 Linux 包 URL");
    }
}

fn url_file_name(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('/')
        .next()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// 由清单里的主 URL 推导 GitHub / Gitee 镜像，去重后并行下载。
fn asset_mirror_urls(primary: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |u: String| {
        if !u.trim().is_empty() && !out.iter().any(|x| x == &u) {
            out.push(u);
        }
    };
    push(primary.trim().to_string());
    if let Some(rest) = primary.strip_prefix(GITEE_REPO_PREFIX) {
        push(format!("{GITHUB_REPO_PREFIX}{rest}"));
    }
    if let Some(rest) = primary.strip_prefix(GITHUB_REPO_PREFIX) {
        push(format!("{GITEE_REPO_PREFIX}{rest}"));
    }
    out
}

/// 查询最新版；若无更新返回 `Ok(None)`。
pub fn check_latest(local_version: &str) -> Result<Option<AvailableUpdate>> {
    let client = http_client()?;
    let m = fetch_manifest(&client)?;
    let ver = m
        .latest_version
        .trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string();
    if ver.is_empty() {
        bail!("更新清单缺少 latest_version");
    }
    if !is_newer(&ver, local_version) {
        info!(
            local = local_version,
            remote = %ver,
            channel = %m.channel,
            "已是最新版"
        );
        let _ = m.min_supported_version;
        return Ok(None);
    }
    let (asset, asset_name) = pick_platform_asset(&m)?;
    let html = if !m.html_url.trim().is_empty() {
        m.html_url
    } else if !m.release_notes_url.trim().is_empty() {
        m.release_notes_url.clone()
    } else {
        format!("https://gitee.com/jian0786/starquake/releases/tag/v{ver}")
    };
    let body = if !m.notes.trim().is_empty() {
        m.notes.trim().to_string()
    } else if !m.release_notes_url.trim().is_empty() {
        format!("发行说明：{}", m.release_notes_url.trim())
    } else {
        String::new()
    };
    Ok(Some(AvailableUpdate {
        version: ver.clone(),
        name: format!("StarQuake {ver}"),
        body,
        html_url: html,
        asset_url: asset.url,
        asset_name,
        sha256: asset.sha256.trim().to_ascii_lowercase(),
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

fn sha256_hex(path: &Path) -> Result<String> {
    // 进程内计算，避免 certutil 输出带空格/本地化文案导致解析失败。
    let mut file = File::open(path).with_context(|| format!("打开 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).context("读取更新包")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn download_one_url(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expect_sha: &str,
) -> Result<()> {
    info!(url = %url, dest = %dest.display(), "下载更新包");
    let mut resp = client.get(url).send().context("下载更新包")?;
    if !resp.status().is_success() {
        bail!("HTTP {}", resp.status());
    }
    let mut file = File::create(dest)?;
    io::copy(&mut resp, &mut file)?;
    file.flush()?;
    if !expect_sha.is_empty() {
        let got = sha256_hex(dest)?;
        if got != expect_sha {
            let _ = fs::remove_file(dest);
            bail!("校验失败（期望 {expect_sha}，实际 {got}）");
        }
        info!(url = %url, "更新包 SHA256 校验通过");
    }
    Ok(())
}

/// 下载资源到暂存目录，返回本地文件路径。
/// GitHub / Gitee 镜像并行下载，先成功（且校验通过）者优先。
pub fn download_asset(info: &AvailableUpdate) -> Result<PathBuf> {
    let dir = staging_root()?;
    let dest = dir.join(&info.asset_name);
    let urls = asset_mirror_urls(&info.asset_url);
    if urls.is_empty() {
        bail!("更新包 URL 为空");
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()?;

    let (tx, rx) = mpsc::channel();
    let n = urls.len();
    for (i, url) in urls.into_iter().enumerate() {
        let tx = tx.clone();
        let client = client.clone();
        let part = dir.join(format!("part_{i}_{}", info.asset_name));
        let sha = info.sha256.clone();
        std::thread::spawn(move || {
            let result = download_one_url(&client, &url, &part, &sha).map(|_| (url, part));
            let _ = tx.send(result);
        });
    }
    drop(tx);

    let mut errors = Vec::new();
    let mut remaining = n;
    while remaining > 0 {
        match rx.recv() {
            Ok(Ok((url, part))) => {
                if part != dest {
                    fs::rename(&part, &dest)
                        .or_else(|_| {
                            fs::copy(&part, &dest)?;
                            fs::remove_file(&part)?;
                            Ok::<(), io::Error>(())
                        })
                        .with_context(|| format!("落盘 {}", dest.display()))?;
                }
                info!(url = %url, dest = %dest.display(), "更新包下载完成");
                return Ok(dest);
            }
            Ok(Err(e)) => {
                errors.push(format!("{e:#}"));
                remaining -= 1;
            }
            Err(_) => break,
        }
    }
    bail!(
        "下载失败：{}",
        if errors.is_empty() {
            "无可用镜像".into()
        } else {
            errors.join("；")
        }
    )
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
    let status = Command::new("tar")
        .args([
            "-xzf",
            &path.to_string_lossy(),
            "-C",
            &dest.to_string_lossy(),
        ])
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

    #[test]
    fn parse_manifest_shape() {
        let raw = r#"{
          "latest_version": "2.0.1",
          "notes": "hello",
          "zip": { "url": "https://example.com/a.zip", "sha256": "" },
          "installer": { "url": "https://example.com/a.exe", "sha256": "" }
        }"#;
        let m: UpdateManifest = serde_json::from_str(raw).unwrap();
        assert_eq!(m.latest_version, "2.0.1");
        assert_eq!(m.notes, "hello");
        assert!(m.zip.unwrap().url.contains("a.zip"));
    }

    #[test]
    fn mirror_urls_gitee_and_github() {
        let gitee = "https://gitee.com/jian0786/starquake/releases/download/v2.2.1/a.zip";
        let urls = asset_mirror_urls(gitee);
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|u| u.contains("github.com/Jian11323")));
        assert!(urls.iter().any(|u| u.contains("gitee.com/jian0786")));

        let github = "https://github.com/Jian11323/StarQuake/releases/download/v2.2.1/a.zip";
        let urls2 = asset_mirror_urls(github);
        assert_eq!(urls2.len(), 2);
    }
}
